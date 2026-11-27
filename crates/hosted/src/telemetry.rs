//! Explicit, bounded OTLP/HTTP JSON metrics export. No environment-driven
//! endpoints, arbitrary labels, audit identity fields, or payload data.
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::config::{https_url, SecretRef};
use crate::error::{HostedError, Result};
use crate::metrics::Snapshot;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_AUTHORIZATION_BYTES: usize = 8 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TelemetryConfig {
    Disabled {},
    OtlpHttp {
        /// The full HTTPS metrics endpoint, normally ending in /v1/metrics.
        endpoint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token: Option<SecretRef>,
        #[serde(default = "default_interval")]
        interval_seconds: u64,
        #[serde(default = "default_timeout")]
        timeout_milliseconds: u64,
    },
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self::Disabled {}
    }
}

fn default_interval() -> u64 {
    30
}

fn default_timeout() -> u64 {
    2000
}

impl TelemetryConfig {
    pub fn validate(&self) -> Result<()> {
        if let Self::OtlpHttp {
            endpoint,
            bearer_token,
            interval_seconds,
            timeout_milliseconds,
        } = self
        {
            if endpoint.len() > 2048
                || !(5..=300).contains(interval_seconds)
                || !(100..=5000).contains(timeout_milliseconds)
            {
                return Err(HostedError::Config("telemetry limits are invalid"));
            }
            https_url(endpoint)?;
            if let Some(secret) = bearer_token {
                secret.validate()?;
            }
        }
        Ok(())
    }
}

pub(crate) struct OtlpExporter {
    client: reqwest::Client,
    endpoint: url::Url,
    interval: Duration,
    timeout: Duration,
}

impl OtlpExporter {
    /// Resolving an explicitly configured credential occurs only for serve.
    /// Disabled telemetry neither reads credentials nor constructs a client.
    pub(crate) fn from_config(config: &TelemetryConfig) -> Result<Option<Self>> {
        config.validate()?;
        let TelemetryConfig::OtlpHttp {
            endpoint,
            bearer_token,
            interval_seconds,
            timeout_milliseconds,
        } = config
        else {
            return Ok(None);
        };
        let endpoint = https_url(endpoint)?;
        let token = bearer_token.as_ref().map(SecretRef::resolve).transpose()?;
        Self::build(
            endpoint,
            token.as_deref(),
            Duration::from_secs(*interval_seconds),
            Duration::from_millis(*timeout_milliseconds),
        )
        .map(Some)
    }

    fn build(
        endpoint: url::Url,
        bearer_token: Option<&str>,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(token) = bearer_token {
            if token.is_empty()
                || token.len() > MAX_AUTHORIZATION_BYTES
                || token
                    .bytes()
                    .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            {
                return Err(HostedError::Config("telemetry credential is invalid"));
            }
            let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| HostedError::Config("telemetry credential is invalid"))?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .https_only(endpoint.scheme() == "https")
            .connect_timeout(timeout.min(Duration::from_secs(1)))
            .timeout(timeout)
            .default_headers(headers)
            .build()
            .map_err(|_| HostedError::Config("telemetry client is unavailable"))?;
        Ok(Self {
            client,
            endpoint,
            interval,
            timeout,
        })
    }

    pub(crate) fn interval(&self) -> Duration {
        self.interval
    }

    pub(crate) async fn export(
        &self,
        snapshot: &Snapshot,
    ) -> std::result::Result<(), &'static str> {
        let payload = serde_json::to_vec(&snapshot.otlp_payload()).map_err(|_| "encoding")?;
        if payload.len() > MAX_REQUEST_BYTES {
            return Err("request_limit");
        }
        // One deadline includes connection, response headers and the entire
        // bounded response. Never retry here or wait in an artifact request.
        tokio::time::timeout(self.timeout, async {
            let response = self
                .client
                .post(self.endpoint.clone())
                .body(payload)
                .send()
                .await
                .map_err(|_| "network")?;
            if response.status() != reqwest::StatusCode::OK {
                return Err("status");
            }
            let is_json = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
                });
            if !is_json {
                return Err("response_type");
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
            {
                return Err("response_limit");
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| "network")?;
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err("response_limit");
                }
                bytes.extend_from_slice(&chunk);
            }
            let response: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| "response")?;
            if !response.is_object() {
                return Err("response");
            }
            if let Some(partial) = response.get("partialSuccess") {
                if !partial.is_object() {
                    return Err("response");
                }
                if let Some(rejected) = partial.get("rejectedDataPoints") {
                    let count = rejected
                        .as_u64()
                        .or_else(|| rejected.as_str()?.parse::<u64>().ok())
                        .ok_or("response")?;
                    if count > 0 {
                        return Err("partial_success");
                    }
                }
                if let Some(message) = partial.get("errorMessage") {
                    if !message.is_string() {
                        return Err("response");
                    }
                    if message.as_str().is_some_and(|value| !value.is_empty()) {
                        return Err("partial_warning");
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| "timeout")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::{routing::post, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn disabled_is_default_and_configuration_is_strict() {
        assert!(OtlpExporter::from_config(&TelemetryConfig::default())
            .unwrap()
            .is_none());
        for endpoint in [
            "http://collector.example/v1/metrics",
            "https://user:secret@collector.example",
            "https://collector.example/?secret=x",
            "https://collector.example/#fragment",
        ] {
            let value = json!({"mode":"otlp_http", "endpoint":endpoint});
            let config: TelemetryConfig = serde_json::from_value(value).unwrap();
            assert!(config.validate().is_err());
        }
        assert!(serde_json::from_value::<TelemetryConfig>(
            json!({"mode":"disabled", "endpoint":"https://collector.example"})
        )
        .is_err());
        let config: TelemetryConfig = serde_json::from_value(
            json!({"mode":"otlp_http", "endpoint":"https://collector.example/v1/metrics"}),
        )
        .unwrap();
        assert!(config.validate().is_ok());
        for (field, value) in [
            ("interval_seconds", 0),
            ("interval_seconds", 301),
            ("timeout_milliseconds", 99),
            ("timeout_milliseconds", 5001),
        ] {
            let mut value_json = serde_json::to_value(&config).unwrap();
            value_json[field] = json!(value);
            assert!(serde_json::from_value::<TelemetryConfig>(value_json)
                .unwrap()
                .validate()
                .is_err());
        }
    }

    async fn collector(app: Router) -> (url::Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            url::Url::parse(&format!("http://{address}/v1/metrics")).unwrap(),
            task,
        )
    }

    #[tokio::test]
    async fn collector_receives_bounded_otlp_cumulative_counters_and_dedicated_auth() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = Router::new().route(
            "/v1/metrics",
            post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let calls = observed.clone();
                async move {
                    assert_eq!(headers[CONTENT_TYPE], "application/json");
                    assert_eq!(headers[AUTHORIZATION], "Bearer collector-only-secret");
                    assert!(body.len() <= MAX_REQUEST_BYTES);
                    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let metrics = payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
                        .as_array()
                        .unwrap();
                    assert_eq!(metrics.len(), 2);
                    for metric in metrics {
                        assert_eq!(metric["sum"]["aggregationTemporality"], 2);
                        assert_eq!(metric["sum"]["isMonotonic"], true);
                        for point in metric["sum"]["dataPoints"].as_array().unwrap() {
                            assert!(point["asInt"].as_str().unwrap().parse::<i64>().is_ok());
                            assert!(
                                point["timeUnixNano"]
                                    .as_str()
                                    .unwrap()
                                    .parse::<u64>()
                                    .unwrap()
                                    >= point["startTimeUnixNano"]
                                        .as_str()
                                        .unwrap()
                                        .parse::<u64>()
                                        .unwrap()
                            );
                        }
                    }
                    assert!(!String::from_utf8_lossy(&body).contains("collector-only-secret"));
                    calls.fetch_add(1, Ordering::SeqCst);
                    ([(CONTENT_TYPE, "application/json")], "{}")
                }
            }),
        );
        let (endpoint, task) = collector(app).await;
        let exporter = OtlpExporter::build(
            endpoint,
            Some("collector-only-secret"),
            Duration::from_secs(30),
            Duration::from_secs(1),
        )
        .unwrap();
        exporter.export(&crate::metrics::snapshot()).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn redirects_partial_rejections_oversize_and_stalls_are_bounded_failures() {
        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let observed = redirect_hits.clone();
        let app = Router::new()
            .route(
                "/v1/metrics",
                post(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/secret-target")],
                    )
                }),
            )
            .route(
                "/secret-target",
                post(move || {
                    let observed = observed.clone();
                    async move {
                        observed.fetch_add(1, Ordering::SeqCst);
                        "{}"
                    }
                }),
            );
        let (endpoint, task) = collector(app).await;
        let exporter = OtlpExporter::build(
            endpoint,
            Some("secret"),
            Duration::from_secs(30),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            exporter.export(&crate::metrics::snapshot()).await,
            Err("status")
        );
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 0);
        task.abort();

        for (response, expected) in [
            (
                "{\"partialSuccess\":{\"rejectedDataPoints\":\"1\",\"errorMessage\":\"secret\"}}"
                    .to_string(),
                "partial_success",
            ),
            (
                "{\"partialSuccess\":{\"rejectedDataPoints\":\"0\",\"errorMessage\":\"secret-warning\"}}".to_string(),
                "partial_warning",
            ),
            ("x".repeat(MAX_RESPONSE_BYTES + 1), "response_limit"),
            ("secret-not-json".to_string(), "response"),
        ] {
            let (endpoint, task) = collector(Router::new().route(
                "/v1/metrics",
                post(move || {
                    let response = response.clone();
                    async move { ([(CONTENT_TYPE, "application/json")], response) }
                }),
            ))
            .await;
            let exporter = OtlpExporter::build(
                endpoint,
                None,
                Duration::from_secs(30),
                Duration::from_secs(1),
            )
            .unwrap();
            assert_eq!(
                exporter.export(&crate::metrics::snapshot()).await,
                Err(expected)
            );
            task.abort();
        }
        let (endpoint, task) = collector(Router::new().route(
            "/v1/metrics",
            post(|| async { std::future::pending::<String>().await }),
        ))
        .await;
        let exporter = OtlpExporter::build(
            endpoint,
            None,
            Duration::from_secs(30),
            Duration::from_millis(100),
        )
        .unwrap();
        let result = exporter.export(&crate::metrics::snapshot()).await;
        assert!(matches!(result, Err("timeout" | "network")));
        task.abort();
    }
}
