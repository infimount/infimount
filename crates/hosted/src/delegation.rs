//! An optional platform URL issuer. It receives no storage or user authority.
//!
//! The ordinary authenticated download grant remains the only redemption
//! authority, and is not published until the issuer's bounded response passes
//! validation. Returned URLs are presentation links, never bearer grants.
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::config::SecretRef;
use crate::error::{HostedError, Result};

const MAX_URL_BYTES: usize = 2048;
const MAX_REQUEST_BYTES: usize = 4096;
const MAX_RESPONSE_BYTES: usize = 8192;
const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryConfig {
    Proxy {},
    Delegated {
        issuer_url: String,
        issuer_token: SecretRef,
        returned_url_origin: String,
        #[serde(default = "default_timeout_milliseconds")]
        timeout_milliseconds: u64,
    },
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self::Proxy {}
    }
}

fn default_timeout_milliseconds() -> u64 {
    2000
}

impl DeliveryConfig {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Proxy {} => Ok(()),
            Self::Delegated {
                issuer_url,
                issuer_token,
                returned_url_origin,
                timeout_milliseconds,
            } => {
                let issuer = https_url(issuer_url)?;
                if issuer.query().is_some() {
                    return Err(HostedError::Config("issuer URL cannot contain a query"));
                }
                let origin = https_url(returned_url_origin)?;
                if origin.origin().ascii_serialization() != *returned_url_origin {
                    return Err(HostedError::Config(
                        "returned URL origin must be a canonical HTTPS origin",
                    ));
                }
                if !(100..=5000).contains(timeout_milliseconds) {
                    return Err(HostedError::Config(
                        "issuer timeout must be between 100 and 5000 milliseconds",
                    ));
                }
                issuer_token.validate()
            }
        }
    }
}

fn https_url(value: &str) -> Result<reqwest::Url> {
    if value.is_empty()
        || value.len() > MAX_URL_BYTES
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        || value.contains('\\')
    {
        return Err(HostedError::Config("delegation URL is invalid"));
    }
    let url =
        reqwest::Url::parse(value).map_err(|_| HostedError::Config("delegation URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(HostedError::Config(
            "delegation URL must be HTTPS without credentials or fragment",
        ));
    }
    Ok(url)
}

#[derive(Clone)]
pub struct DelegatedIssuer {
    client: reqwest::Client,
    issuer_url: reqwest::Url,
    authorization: HeaderValue,
    returned_url_origin: String,
    timeout: Duration,
}

/// This allowlist is the entire webhook contract. No path, storage identifier,
/// backend endpoint, principal, access token, content or checksum is forwarded.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuerRequest {
    version: u8,
    grant_id: String,
    redeem_url: String,
    expires_at_unix_seconds: u64,
    requires_authorization: bool,
    single_use: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IssuerResponse {
    version: u8,
    grant_id: String,
    url: String,
    expires_at_unix_seconds: u64,
}

impl DelegatedIssuer {
    /// Construct without making a network request. Disabled delivery does not
    /// resolve a credential, build a client, or contact any issuer.
    pub fn from_config(config: &DeliveryConfig) -> Result<Option<Self>> {
        config.validate()?;
        let DeliveryConfig::Delegated {
            issuer_url,
            issuer_token,
            returned_url_origin,
            timeout_milliseconds,
        } = config
        else {
            return Ok(None);
        };
        let token = issuer_token.resolve()?;
        if token.len() < 16
            || token.len() > 4096
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
        {
            return Err(HostedError::Config(
                "issuer credential must be a bounded bearer token",
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| HostedError::Config("issuer credential is invalid"))?;
        authorization.set_sensitive(true);
        let timeout = Duration::from_millis(*timeout_milliseconds);
        let client = reqwest::Client::builder()
            .https_only(true)
            .http1_only()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| HostedError::Config("issuer HTTP client is unavailable"))?;
        Ok(Some(Self {
            client,
            issuer_url: https_url(issuer_url)?,
            authorization,
            returned_url_origin: returned_url_origin.clone(),
            timeout,
        }))
    }

    pub(crate) async fn issue(
        &self,
        grant_id: &str,
        redeem_url: &str,
        expires_at_unix_seconds: u64,
        grant_deadline: Instant,
    ) -> Result<String> {
        if expires_at_unix_seconds <= crate::now_epoch_seconds() || grant_deadline <= Instant::now()
        {
            return Err(HostedError::NotFound);
        }
        let request = IssuerRequest {
            version: 1,
            grant_id: grant_id.to_owned(),
            redeem_url: redeem_url.to_owned(),
            expires_at_unix_seconds,
            requires_authorization: true,
            single_use: true,
        };
        let body = serde_json::to_vec(&request).map_err(|_| HostedError::Internal)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(HostedError::Limit);
        }
        let deadline = grant_deadline.min(Instant::now() + self.timeout);
        let response = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let response = self
                .client
                .post(self.issuer_url.clone())
                .header(AUTHORIZATION, self.authorization.clone())
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json")
                .header(CACHE_CONTROL, "no-store")
                .body(body)
                .send()
                .await
                .map_err(|_| HostedError::Unavailable)?;
            if response.status() != reqwest::StatusCode::OK
                || response
                    .content_length()
                    .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
                || response.headers().len() > 32
                || response
                    .headers()
                    .iter()
                    .fold(0usize, |total, (name, value)| {
                        total
                            .saturating_add(name.as_str().len())
                            .saturating_add(value.as_bytes().len())
                            .saturating_add(4)
                    })
                    > MAX_HEADER_BYTES
                || response.headers().get_all(CONTENT_TYPE).iter().count() != 1
                || !response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        matches!(
                            value,
                            "application/json" | "application/json; charset=utf-8"
                        )
                    })
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_ENCODING)
            {
                return Err(HostedError::Unavailable);
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| HostedError::Unavailable)?;
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(HostedError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            let response: IssuerResponse =
                serde_json::from_slice(&bytes).map_err(|_| HostedError::Unavailable)?;
            self.validate_response(&request, response)
        })
        .await
        .map_err(|_| HostedError::Unavailable)??;
        if expires_at_unix_seconds <= crate::now_epoch_seconds() || grant_deadline <= Instant::now()
        {
            return Err(HostedError::NotFound);
        }
        Ok(response)
    }

    fn validate_response(
        &self,
        request: &IssuerRequest,
        response: IssuerResponse,
    ) -> Result<String> {
        if response.version != request.version
            || response.grant_id != request.grant_id
            || response.expires_at_unix_seconds != request.expires_at_unix_seconds
        {
            return Err(HostedError::Unavailable);
        }
        let url = https_url(&response.url).map_err(|_| HostedError::Unavailable)?;
        if url.origin().ascii_serialization() != self.returned_url_origin
            || url.as_str() != response.url
        {
            return Err(HostedError::Unavailable);
        }
        Ok(response.url)
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use axum::body::{Body, Bytes};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    pub const TOKEN: &str = "dedicated-issuer-fixture-credential";
    pub const ORIGIN: &str = "https://platform.example";

    #[derive(Clone, Copy)]
    pub enum Mode {
        Valid,
        Paused,
        Failure,
        WrongOrigin,
        WrongGrant,
        ChangedExpiry,
        UnknownField,
        DuplicateField,
        InvalidJson,
        WrongContentType,
        TooLarge,
        LargeHeader,
        Redirect,
    }

    #[derive(Clone)]
    pub struct Recorded {
        pub body: Value,
        pub authorization: String,
    }

    struct FixtureState {
        mode: Mode,
        requests: Arc<Mutex<Vec<Recorded>>>,
        started: Arc<Notify>,
        release: Arc<Notify>,
        redirects: Arc<AtomicUsize>,
        redirect_url: String,
    }

    pub struct MockIssuer {
        pub issuer: DelegatedIssuer,
        pub requests: Arc<Mutex<Vec<Recorded>>>,
        pub started: Arc<Notify>,
        pub release: Arc<Notify>,
        pub redirects: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for MockIssuer {
        fn drop(&mut self) {
            self.release.notify_waiters();
            self.task.abort();
        }
    }

    impl MockIssuer {
        pub async fn start(mode: Mode, timeout: Duration) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/issue", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let started = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let redirects = Arc::new(AtomicUsize::new(0));
            let state = Arc::new(FixtureState {
                mode,
                requests: requests.clone(),
                started: started.clone(),
                release: release.clone(),
                redirects: redirects.clone(),
                redirect_url: format!("http://{}/redirected", listener.local_addr().unwrap()),
            });
            let app = Router::new()
                .route("/issue", post(issue))
                .route("/redirected", post(redirected))
                .with_state(state);
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            // Test-only construction permits an in-process HTTP fixture. The
            // production constructor above always enforces HTTPS and TLS.
            let mut authorization = HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap();
            authorization.set_sensitive(true);
            let issuer = DelegatedIssuer {
                client: reqwest::Client::builder()
                    .http1_only()
                    .no_proxy()
                    .retry(reqwest::retry::never())
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(timeout)
                    .build()
                    .unwrap(),
                issuer_url: reqwest::Url::parse(&endpoint).unwrap(),
                authorization,
                returned_url_origin: ORIGIN.into(),
                timeout,
            };
            Self {
                issuer,
                requests,
                started,
                release,
                redirects,
                task,
            }
        }

        pub async fn wait_started(&self) {
            tokio::time::timeout(Duration::from_secs(2), self.started.notified())
                .await
                .unwrap();
        }
    }

    async fn redirected(State(state): State<Arc<FixtureState>>) -> Response {
        state.redirects.fetch_add(1, Ordering::SeqCst);
        Json(json!({"unexpected":"redirect followed"})).into_response()
    }

    async fn issue(
        State(state): State<Arc<FixtureState>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let request: Value = serde_json::from_slice(&body).unwrap();
        state.requests.lock().unwrap().push(Recorded {
            body: request.clone(),
            authorization: headers.get(AUTHORIZATION).unwrap().to_str().unwrap().into(),
        });
        state.started.notify_one();
        if matches!(state.mode, Mode::Paused) {
            let _ = tokio::time::timeout(Duration::from_secs(10), state.release.notified()).await;
        }
        let mut response = json!({
            "version":request["version"],
            "grant_id":request["grant_id"],
            "url":format!("{ORIGIN}/artifacts/{}", request["grant_id"].as_str().unwrap()),
            "expires_at_unix_seconds":request["expires_at_unix_seconds"]
        });
        match state.mode {
            Mode::Valid | Mode::Paused => Json(response).into_response(),
            Mode::Failure => (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error":"never-log-issuer-response"})),
            )
                .into_response(),
            Mode::WrongOrigin => {
                response["url"] = json!("https://untrusted.example/download");
                Json(response).into_response()
            }
            Mode::WrongGrant => {
                response["grant_id"] = json!(uuid::Uuid::new_v4().to_string());
                Json(response).into_response()
            }
            Mode::ChangedExpiry => {
                response["expires_at_unix_seconds"] =
                    json!(request["expires_at_unix_seconds"].as_u64().unwrap() + 1);
                Json(response).into_response()
            }
            Mode::UnknownField => {
                response["access_token"] = json!("never-log-issuer-response");
                Json(response).into_response()
            }
            Mode::DuplicateField => {
                let text = serde_json::to_string(&response).unwrap();
                (
                    [(CONTENT_TYPE, "application/json")],
                    format!("{{\"version\":1,{}", &text[1..]),
                )
                    .into_response()
            }
            Mode::InvalidJson => (
                [(CONTENT_TYPE, "application/json")],
                "never-log-issuer-response",
            )
                .into_response(),
            Mode::WrongContentType => (
                [(CONTENT_TYPE, "text/html")],
                serde_json::to_string(&response).unwrap(),
            )
                .into_response(),
            Mode::TooLarge => {
                let body = Body::from_stream(futures::stream::iter(vec![
                    Ok::<_, std::io::Error>(Bytes::from(vec![b' '; MAX_RESPONSE_BYTES])),
                    Ok(Bytes::from_static(b" {}")),
                ]));
                Response::builder()
                    .header(CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap()
            }
            Mode::LargeHeader => {
                let mut response = Json(response).into_response();
                response.headers_mut().insert(
                    "x-oversized",
                    HeaderValue::from_str(&"x".repeat(MAX_HEADER_BYTES)).unwrap(),
                );
                response
            }
            Mode::Redirect => Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(reqwest::header::LOCATION, &state.redirect_url)
                .body(Body::empty())
                .unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn config() -> serde_json::Value {
        json!({"mode":"delegated","issuer_url":"https://issuer.example/issue",
            "issuer_token":{"kind":"env","name":"INFIMOUNT_TEST_ISSUER_KEY"},
            "returned_url_origin":"https://platform.example"})
    }

    #[test]
    fn disabled_delivery_is_default_and_strict_configuration_is_opt_in() {
        assert!(matches!(
            DeliveryConfig::default(),
            DeliveryConfig::Proxy {}
        ));
        assert!(DelegatedIssuer::from_config(&DeliveryConfig::default())
            .unwrap()
            .is_none());
        let configured: DeliveryConfig = serde_json::from_value(config()).unwrap();
        configured.validate().unwrap();
        assert!(serde_json::from_value::<DeliveryConfig>(
            json!({"mode":"proxy","issuer_url":"https://bad.example"})
        )
        .is_err());
        let mut unknown = config();
        unknown["credential"] = json!("inline secret");
        assert!(serde_json::from_value::<DeliveryConfig>(unknown).is_err());
        for (field, value) in [
            ("issuer_url", "http://issuer.example/issue"),
            ("issuer_url", "https://user:password@issuer.example/issue"),
            ("issuer_url", "https://issuer.example/issue?token=secret"),
            ("issuer_url", "https://issuer.example/issue#fragment"),
            ("issuer_url", "https://issuer.example/issue\n"),
            ("returned_url_origin", "https://platform.example/path"),
            ("returned_url_origin", "https://platform.example/"),
            ("returned_url_origin", "https://platform.example:443"),
            ("returned_url_origin", "http://platform.example"),
        ] {
            let mut value_config = config();
            value_config[field] = json!(value);
            assert!(
                serde_json::from_value::<DeliveryConfig>(value_config)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
        for timeout in [0, 99, 5001, u64::MAX] {
            let mut value = config();
            value["timeout_milliseconds"] = json!(timeout);
            assert!(serde_json::from_value::<DeliveryConfig>(value)
                .unwrap()
                .validate()
                .is_err());
        }
    }

    #[test]
    fn service_credential_is_explicit_bounded_and_sensitive() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut value = config();
        value["issuer_token"] = json!({"kind":"file","path":file.path()});
        let config: DeliveryConfig = serde_json::from_value(value).unwrap();
        for token in [
            "short",
            "contains spaces in credential",
            "line\nbreak-in-credential",
        ] {
            std::fs::write(file.path(), token).unwrap();
            assert!(DelegatedIssuer::from_config(&config).is_err());
        }
        std::fs::write(file.path(), fixture::TOKEN).unwrap();
        let issuer = DelegatedIssuer::from_config(&config).unwrap().unwrap();
        assert!(issuer.authorization.is_sensitive());
    }

    #[tokio::test]
    async fn bounded_typed_exchange_uses_only_dedicated_service_authorization() {
        let mock = fixture::MockIssuer::start(fixture::Mode::Valid, Duration::from_secs(1)).await;
        let id = uuid::Uuid::new_v4().to_string();
        let redeem_url = format!("https://storage.example/downloads/{id}");
        let expires = crate::now_epoch_seconds() + 30;
        let url = mock
            .issuer
            .issue(
                &id,
                &redeem_url,
                expires,
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap();
        assert_eq!(url, format!("{}/artifacts/{id}", fixture::ORIGIN));
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization,
            format!("Bearer {}", fixture::TOKEN)
        );
        assert_eq!(
            requests[0].body,
            json!({"version":1,"grant_id":id,"redeem_url":redeem_url,
            "expires_at_unix_seconds":expires,"requires_authorization":true,"single_use":true})
        );
    }

    #[tokio::test]
    async fn redirects_timeouts_and_unbounded_or_ambiguous_responses_fail_closed() {
        use fixture::Mode;
        for mode in [
            Mode::Failure,
            Mode::WrongOrigin,
            Mode::WrongGrant,
            Mode::ChangedExpiry,
            Mode::UnknownField,
            Mode::DuplicateField,
            Mode::InvalidJson,
            Mode::WrongContentType,
            Mode::TooLarge,
            Mode::LargeHeader,
            Mode::Redirect,
            Mode::Paused,
        ] {
            let mock = fixture::MockIssuer::start(mode, Duration::from_millis(100)).await;
            let id = uuid::Uuid::new_v4().to_string();
            let result = mock
                .issuer
                .issue(
                    &id,
                    &format!("https://storage.example/downloads/{id}"),
                    crate::now_epoch_seconds() + 30,
                    Instant::now() + Duration::from_secs(30),
                )
                .await;
            assert!(matches!(result, Err(HostedError::Unavailable)));
            assert_eq!(mock.redirects.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn returned_origin_and_exact_expiry_cannot_be_changed_by_issuer() {
        let mock = fixture::MockIssuer::start(fixture::Mode::Valid, Duration::from_secs(1)).await;
        let request = IssuerRequest {
            version: 1,
            grant_id: uuid::Uuid::new_v4().to_string(),
            redeem_url: "https://storage.example/downloads/id".into(),
            expires_at_unix_seconds: crate::now_epoch_seconds() + 30,
            requires_authorization: true,
            single_use: true,
        };
        for url in [
            "http://platform.example/artifact",
            "https://platform.example.evil/artifact",
            "https://platform.example:444/artifact",
            "https://platform.example@evil.example/artifact",
            "https://user:secret@platform.example/artifact",
            "https://platform.example/artifact#fragment",
            "https://platform.example/a/../artifact",
            "https://platform.example/artifact\n",
        ] {
            let response = IssuerResponse {
                version: 1,
                grant_id: request.grant_id.clone(),
                url: url.into(),
                expires_at_unix_seconds: request.expires_at_unix_seconds,
            };
            assert!(
                mock.issuer.validate_response(&request, response).is_err(),
                "{url}"
            );
        }
    }
}
