#![allow(non_snake_case)]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use infimount_core::CoreError;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConnectInput {
    pub provider: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub root_path: Option<String>,
    #[serde(default)]
    pub versioning: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConnectOutput {
    pub provider: String,
    pub config: Value,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

fn oauth_storage_config_from_token(
    provider: &str,
    client_id: String,
    client_secret: Option<String>,
    token: OAuthTokenResponse,
    root_path: Option<String>,
    versioning: Option<bool>,
) -> Value {
    let mut config = serde_json::Map::new();
    config.insert("clientId".to_string(), Value::String(client_id));
    if let Some(secret) = client_secret.as_ref() {
        config.insert("clientSecret".to_string(), Value::String(secret.clone()));
    }

    // OpenDAL requires access-token and refresh-token modes to be mutually exclusive.
    // Prefer durable refresh-token configs only when we have the provider-required
    // client credentials to refresh successfully after restart.
    match token.refresh_token {
        Some(refresh_token) if provider == "onedrive" || client_secret.is_some() => {
            config.insert("refreshToken".to_string(), Value::String(refresh_token));
        }
        _ => {
            config.insert("accessToken".to_string(), Value::String(token.access_token));
        }
    }

    if let Some(root) = root_path
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        config.insert("rootPath".to_string(), Value::String(root));
    }
    if provider == "onedrive" {
        config.insert(
            "versioning".to_string(),
            Value::Bool(versioning.unwrap_or(false)),
        );
    }

    Value::Object(config)
}

fn oauth_random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0_u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn oauth_pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn oauth_provider_settings(
    provider: &str,
) -> Result<(&'static str, &'static str, &'static str, &'static str), CoreError> {
    match provider {
        "gdrive" | "google_drive" | "google-drive" => Ok((
            "gdrive",
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://oauth2.googleapis.com/token",
            "https://www.googleapis.com/auth/drive",
        )),
        "onedrive" | "one_drive" | "one-drive" => Ok((
            "onedrive",
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            "Files.ReadWrite offline_access",
        )),
        other => Err(CoreError::Config(format!(
            "unsupported OAuth provider '{other}'"
        ))),
    }
}

const OAUTH_CALLBACK_PATH: &str = "/oauth/callback";
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const OAUTH_CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(10);

async fn wait_for_oauth_callback(
    listener: TcpListener,
    expected_state: String,
) -> Result<String, CoreError> {
    wait_for_oauth_callback_with_timeouts(
        listener,
        expected_state,
        OAUTH_CALLBACK_TIMEOUT,
        OAUTH_CALLBACK_READ_TIMEOUT,
    )
    .await
}

async fn wait_for_oauth_callback_with_timeouts(
    listener: TcpListener,
    expected_state: String,
    accept_timeout: Duration,
    read_timeout: Duration,
) -> Result<String, CoreError> {
    let (mut stream, peer) = tokio::time::timeout(accept_timeout, listener.accept())
        .await
        .map_err(|_| CoreError::Config("OAuth authorization timed out".to_string()))?
        .map_err(CoreError::Io)?;

    if !peer.ip().is_loopback() {
        let _ = stream
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nForbidden")
            .await;
        return Err(CoreError::Config(
            "OAuth callback must come from loopback".to_string(),
        ));
    }

    let mut buf = vec![0_u8; 8192];
    let n = tokio::time::timeout(read_timeout, stream.read(&mut buf))
        .await
        .map_err(|_| CoreError::Config("OAuth callback timed out".to_string()))?
        .map_err(CoreError::Io)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let request_line = request.lines().next().unwrap_or_default();
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != OAUTH_CALLBACK_PATH {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth callback path not found. Return to Infimount and try again.")
            .await;
        return Err(CoreError::Config(
            "OAuth callback path mismatch".to_string(),
        ));
    }
    let params = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| {
            let decoded = urlencoding::decode(value)
                .map(|v| v.to_string())
                .unwrap_or_else(|_| value.to_string());
            (key.to_string(), decoded)
        })
        .collect::<HashMap<_, _>>();

    let state = params.get("state").map(String::as_str).unwrap_or_default();
    if state != expected_state {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth state mismatch. Return to Infimount and try again.")
            .await;
        return Err(CoreError::Config("OAuth state mismatch".to_string()));
    }

    if let Some(error) = params.get("error") {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOAuth authorization was denied or failed. Return to Infimount.")
            .await;
        return Err(CoreError::Config(format!(
            "OAuth authorization failed: {error}"
        )));
    }

    let code = params.get("code").cloned().ok_or_else(|| {
        CoreError::Config("OAuth callback did not include an authorization code".to_string())
    })?;

    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><title>Infimount OAuth Complete</title><body><h1>Infimount connected</h1><p>You can close this window and return to Infimount.</p></body>")
        .await;
    Ok(code)
}

async fn exchange_oauth_token(
    token_endpoint: &str,
    form: &[(&str, String)],
) -> Result<OAuthTokenResponse, CoreError> {
    let response = reqwest::Client::new()
        .post(token_endpoint)
        .form(form)
        .send()
        .await
        .map_err(|_| CoreError::Config("OAuth token exchange failed".to_string()))?;

    if !response.status().is_success() {
        return Err(CoreError::Config(format!(
            "OAuth token exchange failed with provider status {}",
            response.status().as_u16()
        )));
    }

    response
        .json()
        .await
        .map_err(|_| CoreError::Config("OAuth token response could not be parsed".to_string()))
}

#[tauri::command]
pub async fn connect_oauth_storage(
    input: OAuthConnectInput,
) -> Result<OAuthConnectOutput, CoreError> {
    let client_id = input.client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(CoreError::Config("OAuth Client ID is required".to_string()));
    }

    let (provider, auth_endpoint, token_endpoint, scope) =
        oauth_provider_settings(&input.provider)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(CoreError::Io)?;
    let port = listener.local_addr().map_err(CoreError::Io)?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");
    let state = oauth_random_urlsafe(32);
    let verifier = oauth_random_urlsafe(64);
    let challenge = oauth_pkce_challenge(&verifier);

    let mut auth_url = format!(
        "{auth_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(&state),
        urlencoding::encode(&challenge),
    );
    if provider == "gdrive" {
        auth_url.push_str("&access_type=offline&prompt=consent");
    }

    open::that_detached(&auth_url).map_err(|_| {
        CoreError::Config("Failed to open OAuth authorization URL in the browser".to_string())
    })?;

    let code = wait_for_oauth_callback(listener, state).await?;
    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id.clone()),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = input
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        form.push(("client_secret", secret.to_string()));
    }

    let token = exchange_oauth_token(token_endpoint, &form).await?;

    let expires_in = token.expires_in;
    let config = oauth_storage_config_from_token(
        provider,
        client_id,
        input
            .client_secret
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        token,
        input.root_path,
        input.versioning,
    );

    let expires_at =
        expires_in.map(|seconds| (Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339());

    Ok(OAuthConnectOutput {
        provider: provider.to_string(),
        config,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    #[test]
    fn oauth_provider_settings_accepts_drive_aliases() {
        assert_eq!(oauth_provider_settings("google_drive").unwrap().0, "gdrive");
        assert_eq!(oauth_provider_settings("one-drive").unwrap().0, "onedrive");
        assert!(oauth_provider_settings("mystery").is_err());
    }

    #[test]
    fn oauth_pkce_challenge_is_s256_urlsafe() {
        let challenge = oauth_pkce_challenge("verifier");
        assert_eq!(challenge, "iMnq5o6zALKXGivsnlom_0F5_WYda32GHkxlV7mq7hQ");
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn oauth_pkce_verifier_and_state_are_urlsafe_and_high_entropy_length() {
        let verifier = oauth_random_urlsafe(64);
        let state = oauth_random_urlsafe(32);
        let is_unreserved = |value: &str| {
            value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~'))
        };

        assert!((43..=128).contains(&verifier.len()));
        assert!(is_unreserved(&verifier));
        assert!(verifier.len() >= 86);
        assert!(state.len() >= 43);
        assert!(is_unreserved(&state));
        assert_ne!(oauth_random_urlsafe(32), oauth_random_urlsafe(32));
    }

    #[tokio::test]
    async fn oauth_callback_accepts_loopback_code_and_valid_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback(
            listener,
            "expected-state".to_string(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /oauth/callback?code=abc123&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.contains("200 OK"));
        assert_eq!(task.await.unwrap().unwrap(), "abc123");
    }

    #[tokio::test]
    async fn oauth_callback_rejects_state_mismatch_without_returning_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback(
            listener,
            "expected-state".to_string(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /oauth/callback?code=secret-code&state=wrong-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.contains("400 Bad Request"));
        assert!(response.contains("OAuth state mismatch"));
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn oauth_callback_rejects_wrong_path() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback(
            listener,
            "expected-state".to_string(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /wrong?code=abc123&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.contains("404 Not Found"));
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn oauth_callback_times_out_after_silent_loopback_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback_with_timeouts(
            listener,
            "expected-state".to_string(),
            Duration::from_secs(1),
            Duration::from_millis(25),
        ));

        let _stream = TcpStream::connect(addr).await.unwrap();
        let error = task.await.unwrap().unwrap_err().to_string();

        assert!(error.contains("OAuth callback timed out"));
    }

    #[tokio::test]
    async fn oauth_callback_server_closes_after_first_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback(
            listener,
            "expected-state".to_string(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /oauth/callback?code=abc123&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(response.contains("200 OK"));
        assert_eq!(task.await.unwrap().unwrap(), "abc123");

        let second =
            tokio::time::timeout(Duration::from_millis(100), TcpStream::connect(addr)).await;
        assert!(second.is_err() || second.unwrap().is_err());
    }

    #[tokio::test]
    async fn oauth_callback_maps_provider_error_without_secrets() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(wait_for_oauth_callback(
            listener,
            "expected-state".to_string(),
        ));

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET /oauth/callback?error=access_denied&state=expected-state&code=should-not-use HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.contains("400 Bad Request"));
        assert!(!response.contains("should-not-use"));
        assert!(task.await.unwrap().is_err());
    }

    #[test]
    fn oauth_storage_config_uses_mutually_exclusive_token_modes() {
        let google_with_secret = oauth_storage_config_from_token(
            "gdrive",
            "google-client".to_string(),
            Some("google-secret".to_string()),
            OAuthTokenResponse {
                access_token: "access".to_string(),
                refresh_token: Some("refresh".to_string()),
                expires_in: Some(3600),
            },
            Some("/root".to_string()),
            None,
        );
        assert_eq!(google_with_secret["refreshToken"], "refresh");
        assert!(google_with_secret.get("accessToken").is_none());

        let google_without_secret = oauth_storage_config_from_token(
            "gdrive",
            "google-client".to_string(),
            None,
            OAuthTokenResponse {
                access_token: "access".to_string(),
                refresh_token: Some("refresh".to_string()),
                expires_in: Some(3600),
            },
            None,
            None,
        );
        assert_eq!(google_without_secret["accessToken"], "access");
        assert!(google_without_secret.get("refreshToken").is_none());

        let onedrive_public_client = oauth_storage_config_from_token(
            "onedrive",
            "ms-client".to_string(),
            None,
            OAuthTokenResponse {
                access_token: "access".to_string(),
                refresh_token: Some("refresh".to_string()),
                expires_in: Some(3600),
            },
            None,
            Some(true),
        );
        assert_eq!(onedrive_public_client["refreshToken"], "refresh");
        assert_eq!(onedrive_public_client["versioning"], true);
        assert!(onedrive_public_client.get("accessToken").is_none());
    }

    #[tokio::test]
    async fn oauth_token_exchange_accepts_mock_google_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let n = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("POST /token HTTP/1.1"));
            assert!(request.contains("grant_type=authorization_code"));
            assert!(request.contains("code=mock-google-code"));
            assert!(request.contains("code_verifier=mock-verifier"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 91\r\nConnection: close\r\n\r\n{\"access_token\":\"mock-access-token\",\"refresh_token\":\"mock-refresh-token\",\"expires_in\":3600}",
                )
                .await
                .unwrap();
        });

        let token = exchange_oauth_token(
            &endpoint,
            &[
                ("grant_type", "authorization_code".to_string()),
                ("code", "mock-google-code".to_string()),
                (
                    "redirect_uri",
                    "http://127.0.0.1:12345/oauth/callback".to_string(),
                ),
                ("client_id", "mock-client-id".to_string()),
                ("code_verifier", "mock-verifier".to_string()),
            ],
        )
        .await
        .unwrap();

        assert_eq!(token.access_token, "mock-access-token");
        assert_eq!(token.refresh_token.as_deref(), Some("mock-refresh-token"));
        assert_eq!(token.expires_in, Some(3600));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn oauth_token_exchange_accepts_mock_microsoft_response_without_client_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/common/oauth2/v2.0/token",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let n = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("POST /common/oauth2/v2.0/token HTTP/1.1"));
            assert!(request.contains("grant_type=authorization_code"));
            assert!(request.contains("code=mock-microsoft-code"));
            assert!(request.contains("client_id=mock-public-client-id"));
            assert!(request.contains("code_verifier=mock-verifier"));
            assert!(!request.contains("client_secret"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 105\r\nConnection: close\r\n\r\n{\"token_type\":\"Bearer\",\"scope\":\"Files.ReadWrite\",\"expires_in\":3600,\"access_token\":\"mock-ms-access-token\"}",
                )
                .await
                .unwrap();
        });

        let token = exchange_oauth_token(
            &endpoint,
            &[
                ("grant_type", "authorization_code".to_string()),
                ("code", "mock-microsoft-code".to_string()),
                (
                    "redirect_uri",
                    "http://127.0.0.1:12345/oauth/callback".to_string(),
                ),
                ("client_id", "mock-public-client-id".to_string()),
                ("code_verifier", "mock-verifier".to_string()),
            ],
        )
        .await
        .unwrap();

        assert_eq!(token.access_token, "mock-ms-access-token");
        assert_eq!(token.refresh_token, None);
        assert_eq!(token.expires_in, Some(3600));
        server.await.unwrap();
    }

    #[test]
    fn oauth_provider_settings_uses_microsoft_offline_access_scope() {
        let (_, auth_endpoint, token_endpoint, scope) =
            oauth_provider_settings("onedrive").unwrap();
        assert_eq!(
            auth_endpoint,
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
        );
        assert_eq!(
            token_endpoint,
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        assert!(scope.contains("Files.ReadWrite"));
        assert!(scope.contains("offline_access"));
    }

    #[tokio::test]
    async fn oauth_token_exchange_error_does_not_echo_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 62\r\nConnection: close\r\n\r\n{\"error\":\"invalid_grant\",\"secret_code\":\"must-not-leak\"}",
                )
                .await
                .unwrap();
        });

        let error = exchange_oauth_token(
            &endpoint,
            &[
                ("grant_type", "authorization_code".to_string()),
                ("code", "sensitive-auth-code".to_string()),
            ],
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("provider status 400"));
        assert!(!error.contains("must-not-leak"));
        assert!(!error.contains("sensitive-auth-code"));
        server.await.unwrap();
    }
}
