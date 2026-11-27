//! Request-local OAuth access-token verification for the hosted data plane.
//!
//! This verifier accepts the RFC 9068 RS256 `at+jwt` profile only. Identity and
//! tenant claims are matched against server-owned grants, never used as paths.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use futures::StreamExt;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::config::{valid_identity, AuthConfig};
use crate::error::{HostedError, Result};

const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_JWKS_BYTES: usize = 64 * 1024;
const MAX_JWKS_KEYS: usize = 32;
const JWKS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub issuer: String,
    pub subject: String,
    pub tenant: String,
    pub client_id: String,
    pub scopes: BTreeSet<String>,
    pub expires_at: u64,
}

impl Principal {
    pub fn owner_key(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"infimount-hosted-owner-v1\0");
        for value in [&self.issuer, &self.subject, &self.tenant, &self.client_id] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    pub fn ensure_live(&self) -> Result<()> {
        if self.expires_at <= now_seconds()? {
            return Err(HostedError::Unauthorized);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Authenticator {
    inner: Arc<AuthState>,
}

struct AuthState {
    config: AuthConfig,
    client: reqwest::Client,
    cache: Mutex<JwksCache>,
}

struct JwksCache {
    keys: BTreeMap<String, DecodingKey>,
    expires: Instant,
    last_attempt: Instant,
}

#[derive(Deserialize)]
struct AccessClaims {
    iss: String,
    sub: String,
    exp: u64,
    iat: u64,
    #[serde(default)]
    nbf: Option<u64>,
    tenant_id: String,
    client_id: String,
    jti: String,
    scope: String,
    // Explicitly typed as well as independently checked by jsonwebtoken. This
    // rejects wrong-shaped registered claims instead of coercing them.
    aud: Audience,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => {
                !values.is_empty()
                    && values.len() <= 8
                    && values.iter().any(|value| value == expected)
            }
        }
    }
}

impl Authenticator {
    pub async fn new(config: &AuthConfig) -> Result<Self> {
        config.validate()?;
        Self::new_validated(config).await
    }

    async fn new_validated(config: &AuthConfig) -> Result<Self> {
        if jsonwebtoken::crypto::CryptoProvider::try_get_default().is_none() {
            return Err(HostedError::Config(
                "JWT cryptography provider is unavailable",
            ));
        }
        let client = jwks_client()?;
        let keys = fetch_jwks(&client, &config.jwks_url).await?;
        let now = Instant::now();
        let authenticator = Self {
            inner: Arc::new(AuthState {
                config: config.clone(),
                client,
                cache: Mutex::new(JwksCache {
                    keys,
                    expires: now + Duration::from_secs(config.jwks_cache_seconds),
                    last_attempt: now,
                }),
            }),
        };
        authenticator.start_refresh_loop();
        Ok(authenticator)
    }

    /// Cached readiness only. No credentials, tokens or IdP details escape.
    pub fn ready(&self) -> bool {
        self.inner
            .cache
            .try_lock()
            .is_ok_and(|cache| !cache.keys.is_empty() && cache.expires > Instant::now())
    }

    fn start_refresh_loop(&self) {
        let weak = Arc::downgrade(&self.inner);
        let interval = Duration::from_secs(self.inner.config.jwks_cache_seconds / 2);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(inner) = weak.upgrade() else { break };
                let mut cache = inner.cache.lock().await;
                let now = Instant::now();
                if now.duration_since(cache.last_attempt)
                    < Duration::from_secs(inner.config.jwks_refresh_min_seconds)
                {
                    continue;
                }
                cache.last_attempt = now;
                if let Ok(keys) = fetch_jwks(&inner.client, &inner.config.jwks_url).await {
                    cache.keys = keys;
                    cache.expires =
                        Instant::now() + Duration::from_secs(inner.config.jwks_cache_seconds);
                }
                // Failed refresh never extends the old keys' validity. Once
                // expired, all requests and readiness fail closed.
            }
        });
    }

    pub async fn verify(&self, token: &str) -> Result<Principal> {
        if token.is_empty()
            || token.len() > MAX_TOKEN_BYTES
            || token.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(HostedError::Unauthorized);
        }
        let header = decode_header(token).map_err(|_| HostedError::Unauthorized)?;
        if header.alg != Algorithm::RS256
            || !header.typ.as_deref().is_some_and(|value| {
                value.eq_ignore_ascii_case("at+jwt")
                    || value.eq_ignore_ascii_case("application/at+jwt")
            })
            || header.crit.is_some()
            || header.jku.is_some()
            || header.jwk.is_some()
            || header.x5u.is_some()
            || header.enc.is_some()
            || header.zip.is_some()
            || header.cty.is_some()
            || header.extras.inner().contains_key("b64")
        {
            return Err(HostedError::Unauthorized);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| valid_key_id(kid))
            .ok_or(HostedError::Unauthorized)?;
        let key = self.key_for(kid).await?;
        let config = &self.inner.config;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
        validation.set_issuer(&[&config.issuer]);
        validation.set_audience(&[&config.audience]);
        let claims = decode::<AccessClaims>(token, &key, &validation)
            .map_err(|_| HostedError::Unauthorized)?
            .claims;
        let now = now_seconds()?;
        if claims.iss != config.issuer
            || !claims.aud.contains(&config.audience)
            || !valid_identity(&claims.sub)
            || !valid_identity(&claims.tenant_id)
            || !valid_identity(&claims.client_id)
            || !valid_identity(&claims.jti)
            || claims.exp <= now
            || claims.iat > now.saturating_add(30)
            || claims.exp <= claims.iat
            || claims.exp - claims.iat > config.max_token_lifetime_seconds
            || claims.nbf.is_some_and(|nbf| nbf > now || nbf >= claims.exp)
            || claims.scope.len() > 2048
        {
            return Err(HostedError::Unauthorized);
        }
        let scopes = claims
            .scope
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .map(String::from)
            .collect::<BTreeSet<_>>();
        if scopes.is_empty()
            || scopes.len() > 32
            || scopes.iter().any(|scope| {
                scope.len() > 128
                    || !scope
                        .bytes()
                        .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b'"' && byte != b'\\')
            })
        {
            return Err(HostedError::Unauthorized);
        }
        Ok(Principal {
            issuer: claims.iss,
            subject: claims.sub,
            tenant: claims.tenant_id,
            client_id: claims.client_id,
            scopes,
            expires_at: claims.exp,
        })
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey> {
        // Hold one bounded lock across refresh. Concurrent unknown key IDs
        // cannot fan out into an IdP request storm or an unbounded negative cache.
        let mut cache = self.inner.cache.lock().await;
        let now = Instant::now();
        if cache.expires > now {
            if let Some(key) = cache.keys.get(kid) {
                return Ok(key.clone());
            }
        }
        if now.duration_since(cache.last_attempt)
            < Duration::from_secs(self.inner.config.jwks_refresh_min_seconds)
        {
            return Err(if cache.expires <= now {
                HostedError::Unavailable
            } else {
                HostedError::Unauthorized
            });
        }
        cache.last_attempt = now;
        let keys = fetch_jwks(&self.inner.client, &self.inner.config.jwks_url).await?;
        cache.keys = keys;
        cache.expires = Instant::now() + Duration::from_secs(self.inner.config.jwks_cache_seconds);
        cache
            .keys
            .get(kid)
            .cloned()
            .ok_or(HostedError::Unauthorized)
    }
}

fn jwks_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(JWKS_TIMEOUT)
        .connect_timeout(Duration::from_secs(3))
        .https_only(true)
        .build()
        .map_err(|_| HostedError::Unavailable)
}

async fn fetch_jwks(client: &reqwest::Client, url: &str) -> Result<BTreeMap<String, DecodingKey>> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| HostedError::Unavailable)?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|len| len > MAX_JWKS_BYTES as u64)
    {
        return Err(HostedError::Unavailable);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| HostedError::Unavailable)?;
        if body.len().saturating_add(chunk.len()) > MAX_JWKS_BYTES {
            return Err(HostedError::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    parse_jwks(&body)
}

fn parse_jwks(body: &[u8]) -> Result<BTreeMap<String, DecodingKey>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct KeySet {
        keys: Vec<serde_json::Value>,
    }
    if body.len() > MAX_JWKS_BYTES {
        return Err(HostedError::Unavailable);
    }
    let set: KeySet = serde_json::from_slice(body).map_err(|_| HostedError::Unavailable)?;
    if set.keys.is_empty() || set.keys.len() > MAX_JWKS_KEYS {
        return Err(HostedError::Unavailable);
    }
    let mut keys = BTreeMap::new();
    for key in set.keys {
        let kid = key
            .get("kid")
            .and_then(serde_json::Value::as_str)
            .filter(|kid| valid_key_id(kid))
            .ok_or(HostedError::Unavailable)?;
        if key["kty"] != "RSA"
            || key["alg"] != "RS256"
            || key["use"] != "sig"
            || ["d", "p", "q", "dp", "dq", "qi", "oth", "k"]
                .iter()
                .any(|field| key.get(field).is_some())
        {
            return Err(HostedError::Unavailable);
        }
        if let Some(ops) = key.get("key_ops") {
            let ops = ops.as_array().ok_or(HostedError::Unavailable)?;
            if ops.len() != 1 || ops[0] != "verify" {
                return Err(HostedError::Unavailable);
            }
        }
        let n = key
            .get("n")
            .and_then(serde_json::Value::as_str)
            .ok_or(HostedError::Unavailable)?;
        let e = key
            .get("e")
            .and_then(serde_json::Value::as_str)
            .ok_or(HostedError::Unavailable)?;
        let modulus = URL_SAFE_NO_PAD
            .decode(n)
            .map_err(|_| HostedError::Unavailable)?;
        let exponent = URL_SAFE_NO_PAD
            .decode(e)
            .map_err(|_| HostedError::Unavailable)?;
        if !(256..=512).contains(&modulus.len())
            || modulus.first().is_none_or(|byte| byte & 0x80 == 0)
            || modulus.last().is_none_or(|byte| byte & 1 == 0)
            || exponent.is_empty()
            || exponent.len() > 8
            || exponent.first() == Some(&0)
        {
            return Err(HostedError::Unavailable);
        }
        let exponent_value = exponent
            .iter()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
        if exponent_value < 3 || exponent_value & 1 == 0 {
            return Err(HostedError::Unavailable);
        }
        let decoding_key =
            DecodingKey::from_rsa_components(n, e).map_err(|_| HostedError::Unavailable)?;
        if keys.insert(kid.to_string(), decoding_key).is_some() {
            return Err(HostedError::Unavailable);
        }
    }
    Ok(keys)
}

fn valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn now_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| HostedError::Unavailable)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{json, Value};

    // Disposable, public test-only RSA key. Never used by the production verifier.
    const TEST_PRIVATE_DER: &str = "MIIEpAIBAAKCAQEAv+895fN+plCKiKWCdv53RHEl37ulQZrwXRRei5NVrO0pdqKzg6bfW85Zh2Lr8bnus4KGKTmOwwDLYyfilvyJkNfaLPrbkdjngF6HpdTfw/Ca1lZKUbjRMa2w86KO1uE2luiY1i8QKdl1yCB2gyvMzTvAri+05Nsj2Sl7nEl597VeKCrAI4vEDZp+7xJ9x4NDhKH9wTg6xhObQOprBW0qbxedobOVQ5WnZLAxvsbUzl9ViHI/ol5TTVaNVwRh/KcO6Homb1RkeE4LuGP5G7dmkB76vXHGCUyOezAU7Zb6azMiePRj2tNlRGd2HKlCJbvIiCI8p858iy4FyFA82Rjg9QIDAQABAoIBABFFbK3TP25P3md1cqFVPKjtVZnQajUrxWgFe+HWHDuMtn0fyZpAheYzp11wyrKHJ10CkyKv3zzaRshham5pGH1ELzR6TmUivjzlmhHoxJ9TzgXDi2SB27E7Y/+d0noGQhdKTuvXHak7vEbfRPWAwiklsKz79gHcsQPK9n3rJA06HiKRtZcFWUuOpAnqpagrsbdloB9YKUQ7pbI1Yhm+wDLfrErJhjPtIGTA7M9bLON7b7/6DwHTAROULycrB0sFk3DMnvebKcGVNzjPw5MT3+E2fn3j4Gkf2ZTBYLWQ/KJl33cVrIrOc6FIg80PCBiENzSVMhTyUE70xZOnj7NMcnECgYEA8bTQdTSghByXHKTY6JPiLoclq3QUQlPcrKEc0Ycw2+toLT/tz9AF/bttGERWgnOW8/0JRCB5T0mqE58rHzn9IKpU/iWRdKVCDWNQo3RXxBUE0Piyxe74RtNe6cE9Mkjj8N1K8OzHut8rURWzXCrDJ2RKftO70hN2SiharQ09CfcCgYEAy0jvF+jyENimGaCUeMbB7vHwN3YMgLwWNFiX/W45VA7j+MEi+MkFJM31TL9CAfXxZJRrKwgHgGaI4HtiynTHSMdQ+2v9zHxUmNcAKOfRLmK/a9mYd/w4fwUMQfczqNkZV9MeYZplVKE/3BbBHsePlzU/OtABEvFGcKcWO84cEXMCgYBAWJmSSkNdSFs2RfB04kG5j0MJBOsLyVWbI30PjsUEFwRx324I/+FIbigZ6L97FRiftcJMv7Rr1jmP5bTOKFrp4k6kJg42FnSiKcli6msfCVVx6HN3XCn+ePDwnAAVre/4xIDZ6L83UYgvmu5X4A56Iynt36JFx+aa0WlqXnbcewKBgQC6d30xbnDUeRXrPfvSyWZM2sog5XzUfTLU80r/oaKec0Y0AF0m3aWFPBK9Qla227WAMBAZmbTf163IVAWrdfDy1jl3VniooIcs51JzMykSpmu17xPHQ6lDAgtbWvic8f8oKzScDPbT6VFdfdXJeP5Ckgoc1X2WtZCKdwmS9B2U9wKBgQCrJ4xhNcvye5DkdM2C74BLoc64KMyt8rsDygwdWwP4S2yor4TVH6hkumxO8Cp2Iu1JRxK1MOcu0t5wK2kh5gzoj+iXE5RVvRJaRbZTtw1cZfmxdT4zt90SLvjqx9nXfLKZfOmDbqCb5Xqt6nB7YeHo09KvnlmcrJT6DDgUmUpd1A==";
    const TEST_MODULUS: &str = "v-895fN-plCKiKWCdv53RHEl37ulQZrwXRRei5NVrO0pdqKzg6bfW85Zh2Lr8bnus4KGKTmOwwDLYyfilvyJkNfaLPrbkdjngF6HpdTfw_Ca1lZKUbjRMa2w86KO1uE2luiY1i8QKdl1yCB2gyvMzTvAri-05Nsj2Sl7nEl597VeKCrAI4vEDZp-7xJ9x4NDhKH9wTg6xhObQOprBW0qbxedobOVQ5WnZLAxvsbUzl9ViHI_ol5TTVaNVwRh_KcO6Homb1RkeE4LuGP5G7dmkB76vXHGCUyOezAU7Zb6azMiePRj2tNlRGd2HKlCJbvIiCI8p858iy4FyFA82Rjg9Q";

    pub fn config() -> AuthConfig {
        AuthConfig {
            issuer: "https://issuer.example.test".into(),
            audience: "https://storage.example.test/mcp".into(),
            jwks_url: "https://issuer.example.test/jwks".into(),
            max_token_lifetime_seconds: 3600,
            jwks_cache_seconds: 300,
            jwks_refresh_min_seconds: 30,
        }
    }

    pub fn jwks() -> Value {
        json!({"keys":[{"kid":"test-key", "kty":"RSA", "alg":"RS256", "use":"sig", "key_ops":["verify"], "n":TEST_MODULUS, "e":"AQAB"}]})
    }

    pub fn authenticator() -> Authenticator {
        let now = Instant::now();
        Authenticator {
            inner: Arc::new(AuthState {
                config: config(),
                client: jwks_client().unwrap(),
                cache: Mutex::new(JwksCache {
                    keys: parse_jwks(&serde_json::to_vec(&jwks()).unwrap()).unwrap(),
                    expires: now + Duration::from_secs(300),
                    last_attempt: now,
                }),
            }),
        }
    }

    pub fn claims(subject: &str, tenant: &str, client_id: &str, scope: &str) -> Value {
        let now = now_seconds().unwrap();
        json!({"iss":config().issuer,"aud":config().audience,"sub":subject,"tenant_id":tenant,"client_id":client_id,"jti":"synthetic-token-id","iat":now,"nbf":now,"exp":now+600,"scope":scope})
    }

    pub fn signed(claims: &Value, header: Option<Header>) -> String {
        let mut standard = Header::new(Algorithm::RS256);
        standard.typ = Some("at+jwt".into());
        standard.kid = Some("test-key".into());
        let der = base64::engine::general_purpose::STANDARD
            .decode(TEST_PRIVATE_DER)
            .unwrap();
        encode(
            header.as_ref().unwrap_or(&standard),
            claims,
            &EncodingKey::from_rsa_der(&der),
        )
        .unwrap()
    }

    pub fn token(subject: &str, tenant: &str, client_id: &str, scope: &str) -> String {
        signed(&claims(subject, tenant, client_id, scope), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use test_support::{authenticator, claims, config, jwks, signed, token};

    #[tokio::test]
    async fn verified_principal_binds_issuer_user_tenant_client_and_scopes() {
        let auth = authenticator();
        let principal = auth
            .verify(&token(
                "alice",
                "tenant-a",
                "agent",
                "infimount:read infimount:create",
            ))
            .await
            .unwrap();
        assert_eq!(principal.issuer, config().issuer);
        assert_eq!(principal.subject, "alice");
        assert_eq!(principal.tenant, "tenant-a");
        assert_eq!(principal.client_id, "agent");
        assert!(principal.has_scope("infimount:create"));
        assert!(!principal.has_scope("infimount:download"));
        assert!(principal.ensure_live().is_ok());
        assert_eq!(principal.owner_key().len(), 64);
        for (subject, tenant, client) in [
            ("bob", "tenant-a", "agent"),
            ("alice", "tenant-b", "agent"),
            ("alice", "tenant-a", "other"),
        ] {
            let other = auth
                .verify(&token(subject, tenant, client, "infimount:read"))
                .await
                .unwrap();
            assert_ne!(other.owner_key(), principal.owner_key());
        }
        let refreshed = auth
            .verify(&token("alice", "tenant-a", "agent", "infimount:download"))
            .await
            .unwrap();
        assert_eq!(refreshed.owner_key(), principal.owner_key());
    }

    #[tokio::test]
    async fn rejects_wrong_identity_registered_claim_shapes_and_token_times() {
        let auth = authenticator();
        let now = now_seconds().unwrap();
        let invalid = [
            ("iss", json!("https://attacker.example.test")),
            ("iss", json!([config().issuer])),
            ("aud", json!("https://other.example.test/mcp")),
            ("aud", json!(42)),
            ("sub", json!(["alice"])),
            ("sub", json!("")),
            ("tenant_id", Value::Null),
            ("client_id", json!("")),
            ("jti", json!("")),
            ("exp", json!(now)),
            ("exp", json!(now + 7200)),
            ("exp", json!("9999999999")),
            ("iat", json!(now + 300)),
            ("nbf", json!(now + 60)),
            ("scope", json!("infimount:read\ninfimount:create")),
        ];
        for (field, value) in invalid {
            let mut body = claims("alice", "tenant-a", "agent", "infimount:read");
            body[field] = value;
            assert!(
                matches!(
                    auth.verify(&signed(&body, None)).await,
                    Err(HostedError::Unauthorized)
                ),
                "accepted invalid {field}"
            );
        }
        for field in [
            "iss",
            "aud",
            "sub",
            "exp",
            "iat",
            "tenant_id",
            "client_id",
            "jti",
            "scope",
        ] {
            let mut body = claims("alice", "tenant-a", "agent", "infimount:read");
            body.as_object_mut().unwrap().remove(field);
            assert!(
                auth.verify(&signed(&body, None)).await.is_err(),
                "accepted missing {field}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_id_tokens_critical_extensions_key_injection_and_wrong_algorithms() {
        let auth = authenticator();
        let body = claims("alice", "tenant-a", "agent", "infimount:read");
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key".into());
        assert!(auth
            .verify(&signed(&body, Some(header.clone())))
            .await
            .is_err());
        header.typ = Some("at+jwt".into());
        header.crit = Some(vec!["unrecognized".into()]);
        header.extras.insert("unrecognized", true);
        assert!(auth
            .verify(&signed(&body, Some(header.clone())))
            .await
            .is_err());
        header.crit = None;
        header.jku = Some("https://attacker.example.test/jwks".into());
        assert!(auth.verify(&signed(&body, Some(header))).await.is_err());
        let mut hmac_header = Header::new(Algorithm::HS256);
        hmac_header.typ = Some("at+jwt".into());
        hmac_header.kid = Some("test-key".into());
        let hmac = jsonwebtoken::encode(
            &hmac_header,
            &body,
            &EncodingKey::from_secret(b"test-only-secret").unwrap(),
        )
        .unwrap();
        assert!(auth.verify(&hmac).await.is_err());
        let unsigned = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"at+jwt","kid":"test-key"}"#),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&body).unwrap())
        );
        assert!(auth.verify(&unsigned).await.is_err());
        let valid = signed(&body, None);
        let (payload, signature) = valid.rsplit_once('.').unwrap();
        let replacement = if signature.starts_with('A') { 'B' } else { 'A' };
        let tampered = format!("{payload}.{replacement}{}", &signature[1..]);
        assert!(auth.verify(&tampered).await.is_err());
        assert!(auth.verify(&"a".repeat(MAX_TOKEN_BYTES + 1)).await.is_err());
    }

    #[test]
    fn rejects_ambiguous_oversized_or_inappropriate_jwks() {
        let good = jwks();
        assert!(parse_jwks(&serde_json::to_vec(&good).unwrap()).is_ok());
        let key = good["keys"][0].clone();
        let duplicate = json!({"keys":[key.clone(),key.clone()]});
        assert!(parse_jwks(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        for (field, value) in [
            ("kty", json!("oct")),
            ("alg", json!("HS256")),
            ("use", json!("enc")),
            ("key_ops", json!(["sign"])),
            ("n", json!("AQAB")),
            ("e", json!("AA")),
            ("d", json!("private-material")),
        ] {
            let mut changed = good.clone();
            changed["keys"][0][field] = value;
            assert!(
                parse_jwks(&serde_json::to_vec(&changed).unwrap()).is_err(),
                "accepted {field}"
            );
        }
        assert!(parse_jwks(
            &serde_json::to_vec(&json!({"keys":vec![key;MAX_JWKS_KEYS + 1]})).unwrap()
        )
        .is_err());
        assert!(parse_jwks(&vec![b' '; MAX_JWKS_BYTES + 1]).is_err());
    }

    #[tokio::test]
    async fn expired_cache_fails_closed_and_rotation_is_single_flight() {
        use axum::{routing::get, Json, Router};
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_handler = calls.clone();
        let mut rotated = jwks();
        rotated["keys"][0]["kid"] = json!("rotated-key");
        let app = Router::new().route(
            "/jwks",
            get(move || {
                let calls = calls_for_handler.clone();
                let rotated = rotated.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(rotated)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut settings = config();
        settings.jwks_url = format!("http://{address}/jwks");
        // HTTP is confined to this fixture construction. Production new()
        // requires HTTPS and uses a client with https_only(true).
        let now = Instant::now();
        let auth = Authenticator {
            inner: Arc::new(AuthState {
                config: settings,
                client: reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(JWKS_TIMEOUT)
                    .build()
                    .unwrap(),
                cache: Mutex::new(JwksCache {
                    keys: parse_jwks(&serde_json::to_vec(&jwks()).unwrap()).unwrap(),
                    expires: now - Duration::from_secs(1),
                    last_attempt: now,
                }),
            }),
        };
        assert!(!auth.ready());
        assert!(matches!(
            auth.verify(&token("alice", "tenant-a", "agent", "infimount:read"))
                .await,
            Err(HostedError::Unavailable)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        auth.inner.cache.lock().await.last_attempt -= Duration::from_secs(31);
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("rotated-key".into());
        let rotated_token = signed(
            &claims("alice", "tenant-a", "agent", "infimount:read"),
            Some(header),
        );
        let checks = (0..8).map(|_| auth.verify(&rotated_token));
        for result in futures::future::join_all(checks).await {
            assert!(result.is_ok());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(auth.ready());
        assert!(auth
            .verify(&token("alice", "tenant-a", "agent", "infimount:read"))
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn failed_refresh_never_extends_expired_keys() {
        let original = authenticator();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let now = Instant::now();
        let mut config = config();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        config.jwks_url = format!("http://{}/jwks", listener.local_addr().unwrap());
        drop(listener);
        let auth = Authenticator {
            inner: Arc::new(AuthState {
                config,
                client,
                cache: Mutex::new(JwksCache {
                    keys: original.inner.cache.lock().await.keys.clone(),
                    expires: now - Duration::from_secs(1),
                    last_attempt: now - Duration::from_secs(31),
                }),
            }),
        };
        assert!(matches!(
            auth.verify(&token("alice", "tenant-a", "agent", "infimount:read"))
                .await,
            Err(HostedError::Unavailable)
        ));
        assert!(!auth.ready());
        assert!(matches!(
            auth.verify(&token("alice", "tenant-a", "agent", "infimount:read"))
                .await,
            Err(HostedError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn production_constructor_rejects_http_before_any_network_call() {
        let mut config = config();
        config.jwks_url = "http://127.0.0.1:1/jwks".into();
        assert!(matches!(
            Authenticator::new(&config).await,
            Err(HostedError::Config(_))
        ));
    }
}
