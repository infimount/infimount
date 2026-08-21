use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err, map_io_error, McpErrorCode, McpResult};
use crate::registry::default_config_dir;

const MAX_EVENTS: usize = 5_000;
const MAX_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB
const EXPORT_QUEUE_CAPACITY: usize = 128;
const EXPORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const EXPORT_REQUEST_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductEventName {
    AppLaunched,
    OnboardingStarted,
    OnboardingStepCompleted,
    StorageAdded,
    StorageValidationCompleted,
    WorkspaceCreated,
    SidecarVerified,
    ClientConfigPreviewed,
    ClientConfigApplied,
    McpProbeCompleted,
    ActivationCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductEvent {
    pub id: String,
    pub timestamp: String,
    pub name: ProductEventName,
    pub schema_version: u32,
    pub app_version: String,
    pub os_arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_bucket: Option<String>,
}

impl ProductEvent {
    pub fn new(name: ProductEventName) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            name,
            schema_version: 1,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os_arch: build_os_arch(),
            backend_type: None,
            workspace_template: None,
            access_profile: None,
            client_kind: None,
            success: None,
            failure_stage: None,
            error_code: None,
            duration_bucket: None,
        }
    }

    pub fn validate(&self) -> McpResult<()> {
        if self.schema_version != 1 {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "unsupported product event schema",
            ));
        }
        validate_token("event id", &self.id, 96, |c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
        })?;
        validate_token("app version", &self.app_version, 32, |c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')
        })?;
        validate_token("OS/architecture", &self.os_arch, 48, |c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
        })?;
        validate_optional_allowlist(
            "backend type",
            self.backend_type.as_deref(),
            &[
                "local",
                "fs",
                "s3",
                "webdav",
                "azure_blob",
                "gcs",
                "b2",
                "oss",
                "cos",
                "obs",
                "sftp",
                "ftp",
                "gdrive",
                "onedrive",
            ],
        )?;
        validate_optional_allowlist(
            "workspace template",
            self.workspace_template.as_deref(),
            &[
                "coding",
                "writing",
                "research",
                "data-analysis",
                "admin",
                "custom",
            ],
        )?;
        validate_optional_allowlist(
            "access profile",
            self.access_profile.as_deref(),
            &["none", "read_only", "read_write"],
        )?;
        validate_optional_allowlist(
            "client kind",
            self.client_kind.as_deref(),
            &[
                "generic_stdio",
                "claude_code",
                "claude_desktop",
                "cursor",
                "vs_code",
                "open_code",
                "other",
            ],
        )?;
        validate_optional_allowlist(
            "failure stage",
            self.failure_stage.as_deref(),
            &[
                "startup",
                "sidecar_validation",
                "mcp_handshake",
                "mcp_allowed_op",
                "mcp_denial",
                "activation_probe",
                "storage_validation",
                "backup",
                "restore",
            ],
        )?;
        validate_optional_allowlist(
            "duration bucket",
            self.duration_bucket.as_deref(),
            &["fast", "moderate", "slow"],
        )?;
        if let Some(code) = self.error_code.as_deref() {
            validate_token("error code", code, 64, |c| {
                c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'
            })?;
        }
        if chrono::DateTime::parse_from_rfc3339(&self.timestamp).is_err() {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "invalid product event timestamp",
            ));
        }
        Ok(())
    }
}

fn validate_token(
    label: &str,
    value: &str,
    max_len: usize,
    allowed: impl Fn(char) -> bool,
) -> McpResult<()> {
    if value.is_empty() || value.len() > max_len || !value.chars().all(allowed) {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

fn validate_optional_allowlist(
    label: &str,
    value: Option<&str>,
    allowed: &[&str],
) -> McpResult<()> {
    if value.is_some_and(|value| !allowed.contains(&value)) {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

pub struct ProductEventStore {
    path: PathBuf,
    lock_path: PathBuf,
    exporter: NetworkExporter,
    persist: std::sync::atomic::AtomicBool,
}

impl ProductEventStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::with_exporter(path, NetworkExporter::from_persisted_config(), true)
    }

    pub fn with_persistence(path: Option<PathBuf>, persist: bool) -> Self {
        Self::with_exporter(path, NetworkExporter::from_persisted_config(), persist)
    }

    fn with_exporter(path: Option<PathBuf>, exporter: NetworkExporter, persist: bool) -> Self {
        let path = path.unwrap_or_else(default_product_events_path);
        let lock_path = path.with_extension("lock");
        Self {
            path,
            lock_path,
            exporter,
            persist: std::sync::atomic::AtomicBool::new(persist),
        }
    }

    pub fn set_persistence(&self, persist: bool) {
        self.persist
            .store(persist, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn persistence(&self) -> bool {
        self.persist.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn record(&self, event: ProductEvent) -> McpResult<()> {
        event.validate()?;
        if self.persistence() {
            self.flush_batch(std::slice::from_ref(&event))?;
        }
        self.exporter.send(ExportEvent::Product {
            event: Box::new(event),
        });
        Ok(())
    }

    pub fn flush(&self) -> McpResult<()> {
        Ok(())
    }

    fn flush_batch(&self, events: &[ProductEvent]) -> McpResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        lock_file
            .lock_exclusive()
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        for event in events {
            let line = serde_json::to_string(event).map_err(|e| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    format!("failed to serialize event: {e}"),
                )
            })?;
            writeln!(&file, "{line}").map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }

        file.flush()
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        self.rotate_if_needed(&file)?;

        Ok(())
    }

    fn rotate_if_needed(&self, _file: &fs::File) -> McpResult<()> {
        let bytes =
            fs::read(&self.path).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let mut lines = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            .collect::<Vec<_>>();

        let original_count = lines.len();
        let original_bytes: usize = lines.iter().map(|line| line.len()).sum();
        while lines.len() > MAX_EVENTS
            || lines.iter().map(|line| line.len()).sum::<usize>() > MAX_BYTES as usize
        {
            if lines.is_empty() {
                break;
            }
            lines.remove(0);
        }

        if lines.len() == original_count && original_bytes <= MAX_BYTES as usize {
            return Ok(());
        }

        let mut out_file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        for line in lines {
            out_file
                .write_all(line)
                .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
            if !line.ends_with(b"\n") {
                out_file
                    .write_all(b"\n")
                    .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
            }
        }
        out_file
            .flush()
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        Ok(())
    }

    pub fn read_all(&self) -> McpResult<Vec<ProductEvent>> {
        self.flush()?;

        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let mut events = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<ProductEvent>(line) {
                Ok(event) if event.validate().is_ok() => events.push(event),
                _ => continue,
            }
        }
        Ok(events)
    }

    pub fn clear(&self) -> McpResult<()> {
        self.flush()?;
        if self.path.exists() {
            fs::remove_file(&self.path)
                .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn default_product_events_path() -> PathBuf {
    let mut path = default_config_dir();
    path.push("events.jsonl");
    path
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSummary {
    pub app_version: String,
    pub sidecar_version: Option<String>,
    pub sidecar_status: String,
    pub os_arch: String,
    pub keyring_status: String,
    pub config_file_status: String,
    pub schema_versions: HashMap<String, u32>,
    pub storage_count: usize,
    pub backend_counts: HashMap<String, usize>,
    pub exposed_storage_count: usize,
    pub enabled_tools: Vec<String>,
    pub http_bind_category: String,
    pub port_available: bool,
    pub last_error_codes: Vec<String>,
    pub recent_audit_decision_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBundle {
    pub summary: DiagnosticsSummary,
    pub sanitized_errors: Vec<SanitizedError>,
    pub recent_product_events: Vec<ProductEventSummary>,
    pub recent_audit_events: Vec<AuditEventSummary>,
    pub redaction_manifest: RedactionManifest,
    pub checksums: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductEventSummary {
    pub name: ProductEventName,
    pub success: Option<bool>,
    pub failure_stage: Option<String>,
    pub error_code: Option<String>,
    pub duration_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditEventSummary {
    pub tool_name: String,
    pub operation: String,
    pub decision: String,
    pub error_code: Option<String>,
    pub duration_bucket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedError {
    pub stage: String,
    pub error_code: String,
    pub timestamp: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionManifest {
    pub redacted_fields: Vec<String>,
    pub redacted_count: usize,
}

impl DiagnosticsBundle {
    pub fn validate_no_secrets(&self, seed_values: &[&str]) -> Vec<String> {
        let serialized = serde_json::to_value(self).unwrap_or_default();
        let json_str = serde_json::to_string(&serialized).unwrap_or_default();
        seed_values
            .iter()
            .filter(|s| json_str.contains(*s))
            .map(|s| format!("secret leaked: {s}"))
            .collect()
    }
}

pub fn build_os_arch() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    format!("{os}-{arch}")
}

pub fn telemetry_export_enabled(consent_granted: bool, endpoint: Option<&str>) -> bool {
    consent_granted && endpoint.and_then(validated_export_endpoint).is_some()
}

fn validated_export_endpoint(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    let parsed = reqwest::Url::parse(endpoint).ok()?;
    let secure = parsed.scheme() == "https";
    let loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
        });
    if parsed.host_str().is_none()
        || (!secure && !loopback_http)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(parsed.to_string())
}

fn persisted_telemetry_config() -> (bool, Option<String>) {
    let consent_granted = fs::read_to_string(default_config_dir().join("app_settings.json"))
        .ok()
        .and_then(|data| serde_json::from_str::<serde_json::Value>(&data).ok())
        .and_then(|value| value.get("telemetryConsent").cloned())
        .is_some_and(|value| value == "granted" || value == true);
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    (consent_granted, endpoint)
}

/// Initializes the bounded network exporter from persisted consent and the
/// operator-provided endpoint. The return value reports whether export is active.
pub fn init_telemetry() -> bool {
    let (consent_granted, endpoint) = persisted_telemetry_config();
    telemetry_export_enabled(consent_granted, endpoint.as_deref())
}

#[derive(Debug, Clone)]
enum ExportEvent {
    Product {
        event: Box<ProductEvent>,
    },
    ToolCall {
        schema_version: u32,
        tool_name: String,
    },
    Error {
        schema_version: u32,
        error_code: String,
    },
    Latency {
        schema_version: u32,
        tool_name: String,
        duration_bucket: &'static str,
    },
}

fn otlp_signal_endpoint(base: &str, event: &ExportEvent) -> String {
    let signal = if matches!(event, ExportEvent::Product { .. }) {
        "logs"
    } else {
        "metrics"
    };
    let Ok(mut endpoint) = reqwest::Url::parse(base) else {
        return base.to_string();
    };
    let path = endpoint.path().trim_end_matches('/');
    let path = if path.is_empty() {
        format!("/v1/{signal}")
    } else if path.ends_with("/v1/metrics") {
        format!("{}/v1/{signal}", path.trim_end_matches("/v1/metrics"))
    } else if path.ends_with("/v1/logs") {
        format!("{}/v1/{signal}", path.trim_end_matches("/v1/logs"))
    } else {
        format!("{path}/v1/{signal}")
    };
    endpoint.set_path(&path);
    endpoint.to_string()
}

fn otlp_attribute(key: &str, value: impl Into<String>) -> serde_json::Value {
    json!({ "key": key, "value": { "stringValue": value.into() } })
}

fn otlp_payload(event: &ExportEvent) -> serde_json::Value {
    let resource = json!({
        "attributes": [otlp_attribute("service.name", "infimount_mcp")]
    });
    match event {
        ExportEvent::Product { event } => {
            let event_name = serde_json::to_value(&event.name)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "unknown".to_string());
            let mut attributes = vec![
                otlp_attribute("event.schema_version", event.schema_version.to_string()),
                otlp_attribute("app.version", event.app_version.clone()),
                otlp_attribute("os.arch", event.os_arch.clone()),
            ];
            for (key, value) in [
                ("backend.type", event.backend_type.as_ref()),
                ("workspace.template", event.workspace_template.as_ref()),
                ("access.profile", event.access_profile.as_ref()),
                ("client.kind", event.client_kind.as_ref()),
                ("failure.stage", event.failure_stage.as_ref()),
                ("error.code", event.error_code.as_ref()),
                ("duration.bucket", event.duration_bucket.as_ref()),
            ] {
                if let Some(value) = value {
                    attributes.push(otlp_attribute(key, value.clone()));
                }
            }
            if let Some(success) = event.success {
                attributes.push(json!({
                    "key": "success",
                    "value": { "boolValue": success }
                }));
            }
            let time_unix_nano = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
                .ok()
                .and_then(|timestamp| timestamp.timestamp_nanos_opt())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_string());
            json!({
                "resourceLogs": [{
                    "resource": resource,
                    "scopeLogs": [{
                        "scope": { "name": "infimount.product_events", "version": "1" },
                        "logRecords": [{
                            "timeUnixNano": time_unix_nano,
                            "body": { "stringValue": event_name },
                            "attributes": attributes
                        }]
                    }]
                }]
            })
        }
        ExportEvent::ToolCall {
            schema_version,
            tool_name,
        } => otlp_metric_payload(
            resource,
            "infimount.mcp.tool_calls",
            vec![
                otlp_attribute("schema.version", schema_version.to_string()),
                otlp_attribute("tool.name", tool_name.clone()),
            ],
        ),
        ExportEvent::Error {
            schema_version,
            error_code,
        } => otlp_metric_payload(
            resource,
            "infimount.mcp.errors",
            vec![
                otlp_attribute("schema.version", schema_version.to_string()),
                otlp_attribute("error.code", error_code.clone()),
            ],
        ),
        ExportEvent::Latency {
            schema_version,
            tool_name,
            duration_bucket,
        } => otlp_metric_payload(
            resource,
            "infimount.mcp.latency_buckets",
            vec![
                otlp_attribute("schema.version", schema_version.to_string()),
                otlp_attribute("tool.name", tool_name.clone()),
                otlp_attribute("duration.bucket", *duration_bucket),
            ],
        ),
    }
}

fn otlp_metric_payload(
    resource: serde_json::Value,
    name: &str,
    attributes: Vec<serde_json::Value>,
) -> serde_json::Value {
    let time_unix_nano = Utc::now()
        .timestamp_nanos_opt()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_string());
    json!({
        "resourceMetrics": [{
            "resource": resource,
            "scopeMetrics": [{
                "scope": { "name": "infimount_mcp", "version": "1" },
                "metrics": [{
                    "name": name,
                    "sum": {
                        "aggregationTemporality": 1,
                        "isMonotonic": true,
                        "dataPoints": [{
                            "attributes": attributes,
                            "timeUnixNano": time_unix_nano,
                            "asInt": "1"
                        }]
                    }
                }]
            }]
        }]
    })
}

type ExportConnection = Option<(String, SyncSender<ExportEvent>)>;

#[derive(Debug, Clone, Default)]
struct NetworkExporter {
    connection: Arc<Mutex<ExportConnection>>,
    configured_endpoint: Option<String>,
    enforce_persisted_consent: bool,
}

impl NetworkExporter {
    fn from_persisted_config() -> Self {
        Self {
            connection: Arc::default(),
            configured_endpoint: None,
            enforce_persisted_consent: true,
        }
    }

    #[cfg(test)]
    fn configured(consent_granted: bool, endpoint: Option<&str>) -> Self {
        let configured_endpoint = consent_granted
            .then(|| endpoint.and_then(validated_export_endpoint))
            .flatten();
        let exporter = Self {
            connection: Arc::default(),
            configured_endpoint,
            enforce_persisted_consent: false,
        };
        if let Some(endpoint) = exporter.configured_endpoint.clone() {
            let _ = exporter.ensure_connection(endpoint);
        }
        exporter
    }

    fn ensure_connection(&self, endpoint: String) -> Option<SyncSender<ExportEvent>> {
        let mut connection = self.connection.lock().ok()?;
        if let Some((current_endpoint, sender)) = connection.as_ref() {
            if current_endpoint == &endpoint {
                return Some(sender.clone());
            }
        }

        let (sender, receiver) = mpsc::sync_channel::<ExportEvent>(EXPORT_QUEUE_CAPACITY);
        let worker_endpoint = endpoint.clone();
        let worker_checks_consent = self.enforce_persisted_consent;
        std::thread::Builder::new()
            .name("infimount-telemetry-export".into())
            .spawn(move || {
                let Ok(client) = reqwest::blocking::Client::builder()
                    .connect_timeout(EXPORT_CONNECT_TIMEOUT)
                    .timeout(EXPORT_REQUEST_TIMEOUT)
                    .build()
                else {
                    return;
                };
                while let Ok(event) = receiver.recv() {
                    if worker_checks_consent {
                        let (consent, configured_endpoint) = persisted_telemetry_config();
                        let endpoint_still_matches = configured_endpoint
                            .as_deref()
                            .and_then(validated_export_endpoint)
                            .as_deref()
                            == Some(worker_endpoint.as_str());
                        if !consent || !endpoint_still_matches {
                            continue;
                        }
                    }
                    // Telemetry is best effort. The bounded worker never delays tool calls,
                    // and response bodies are intentionally ignored.
                    let endpoint = otlp_signal_endpoint(&worker_endpoint, &event);
                    let payload = otlp_payload(&event);
                    let _ = client.post(endpoint).json(&payload).send();
                }
            })
            .ok()?;
        *connection = Some((endpoint, sender.clone()));
        Some(sender)
    }

    fn send(&self, event: ExportEvent) {
        let endpoint = if self.enforce_persisted_consent {
            let (consent, endpoint) = persisted_telemetry_config();
            if !consent {
                return;
            }
            endpoint.and_then(|value| validated_export_endpoint(&value))
        } else {
            self.configured_endpoint.clone()
        };
        let Some(endpoint) = endpoint else {
            return;
        };
        if let Some(sender) = self.ensure_connection(endpoint) {
            match sender.try_send(event) {
                Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    #[cfg(test)]
    fn is_enabled(&self) -> bool {
        self.connection
            .lock()
            .is_ok_and(|connection| connection.is_some())
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryState {
    exporter: NetworkExporter,
}

impl TelemetryState {
    pub fn new() -> Self {
        Self {
            exporter: NetworkExporter::from_persisted_config(),
        }
    }

    pub fn record_tool_call(&self, tool_name: &str) {
        if is_safe_tool_name(tool_name) {
            self.exporter.send(ExportEvent::ToolCall {
                schema_version: 1,
                tool_name: tool_name.to_string(),
            });
        }
    }

    pub fn record_error(&self, error_code: &str) {
        if is_safe_error_code(error_code) {
            self.exporter.send(ExportEvent::Error {
                schema_version: 1,
                error_code: error_code.to_string(),
            });
        }
    }

    pub fn record_latency(&self, tool_name: &str, elapsed_ms: f64) {
        if is_safe_tool_name(tool_name) && elapsed_ms.is_finite() && elapsed_ms >= 0.0 {
            let duration_bucket = if elapsed_ms < 100.0 {
                "fast"
            } else if elapsed_ms < 1_000.0 {
                "moderate"
            } else {
                "slow"
            };
            self.exporter.send(ExportEvent::Latency {
                schema_version: 1,
                tool_name: tool_name.to_string(),
                duration_bucket,
            });
        }
    }
}

fn is_safe_tool_name(value: &str) -> bool {
    matches!(
        value,
        "list_dir"
            | "stat_path"
            | "read_file"
            | "mkdir"
            | "write_file"
            | "delete_path"
            | "copy_path"
            | "move_path"
            | "search_paths"
            | "list_versions"
            | "read_file_version"
            | "delete_version"
            | "generate_download_link"
            | "session_create"
            | "session_end"
    )
}

fn is_safe_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::sync::mpsc as test_mpsc;
    use tempfile::tempdir;

    #[test]
    fn product_event_constructor_accepts_data_analysis_workspace() {
        let mut event = ProductEvent::new(ProductEventName::WorkspaceCreated);
        event.workspace_template = Some("data-analysis".to_string());
        event.access_profile = Some("read_only".to_string());
        event.success = Some(true);
        event.validate().expect("valid workspace event");
    }

    #[test]
    fn test_event_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let store = ProductEventStore::new(Some(path.clone()));

        let event = ProductEvent {
            id: "test-1".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            name: ProductEventName::AppLaunched,
            schema_version: 1,
            app_version: "0.8.0".to_string(),
            os_arch: "linux-x86_64".to_string(),
            backend_type: None,
            workspace_template: None,
            access_profile: None,
            client_kind: None,
            success: None,
            failure_stage: None,
            error_code: None,
            duration_bucket: None,
        };

        store.record(event).unwrap();
        store.flush().unwrap();

        let events = store.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].name, ProductEventName::AppLaunched));
    }

    #[test]
    fn persistence_can_be_disabled_without_touching_network_consent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let store = ProductEventStore::with_persistence(Some(path.clone()), false);
        let event = ProductEvent::new(ProductEventName::AppLaunched);
        store.record(event).unwrap();
        assert!(!store.persistence());
        assert!(
            !path.exists(),
            "no local event file when persistence is disabled"
        );

        store.set_persistence(true);
        let persisted = ProductEvent::new(ProductEventName::AppLaunched);
        store.record(persisted).unwrap();
        assert!(path.exists(), "persistence resumes when re-enabled");
        assert_eq!(store.read_all().unwrap().len(), 1);
    }

    #[test]
    fn rotation_enforces_count_and_byte_limits_independently() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let store = ProductEventStore::new(Some(path.clone()));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .unwrap();

        let small = serde_json::to_string(&ProductEvent {
            id: "test-1".into(),
            timestamp: Utc::now().to_rfc3339(),
            name: ProductEventName::AppLaunched,
            schema_version: 1,
            app_version: "0.8.0".into(),
            os_arch: "linux-x86_64".into(),
            backend_type: None,
            workspace_template: None,
            access_profile: None,
            client_kind: None,
            success: None,
            failure_stage: None,
            error_code: None,
            duration_bucket: None,
        })
        .unwrap();
        fs::write(&path, format!("{small}\n").repeat(MAX_EVENTS + 1)).unwrap();
        store.rotate_if_needed(&file).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap().lines().count(),
            MAX_EVENTS
        );

        fs::write(&path, format!("{}\n", "x".repeat(64 * 1024)).repeat(100)).unwrap();
        store.rotate_if_needed(&file).unwrap();
        assert!(fs::metadata(&path).unwrap().len() <= MAX_BYTES);
    }

    #[test]
    fn strict_event_schema_rejects_unknown_and_sensitive_values() {
        let unknown = r#"{"id":"x","timestamp":"2026-01-01T00:00:00Z","name":"app_launched","schemaVersion":1,"appVersion":"0.8.0","osArch":"linux-x86_64","endpoint":"https://secret"}"#;
        assert!(serde_json::from_str::<ProductEvent>(unknown).is_err());
        let mut event: ProductEvent = serde_json::from_str(r#"{"id":"x","timestamp":"2026-01-01T00:00:00Z","name":"app_launched","schemaVersion":1,"appVersion":"0.8.0","osArch":"linux-x86_64"}"#).unwrap();
        event.backend_type = Some("customer-bucket/path".into());
        assert!(event.validate().is_err());
        event.backend_type = None;
        event.schema_version = 2;
        assert!(event.validate().is_err());
    }

    #[test]
    fn telemetry_requires_consent_and_valid_endpoint() {
        assert!(!telemetry_export_enabled(
            false,
            Some("https://otel.example")
        ));
        assert!(!telemetry_export_enabled(true, None));
        assert!(!telemetry_export_enabled(true, Some("file:///tmp/events")));
        assert!(!telemetry_export_enabled(
            true,
            Some("http://example.com/events")
        ));
        assert!(!telemetry_export_enabled(
            true,
            Some("https://user:password@example.com/events")
        ));
        assert!(telemetry_export_enabled(true, Some("https://otel.example")));
    }

    #[test]
    fn exporter_posts_strict_events_without_blocking_callers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/events", listener.local_addr().unwrap());
        let (body_sender, body_receiver) = test_mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 2048];
            loop {
                let count = stream.read(&mut buffer).unwrap_or(0);
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request
                    .windows(b"\"stringValue\":\"list_dir\"".len())
                    .any(|window| window == b"\"stringValue\":\"list_dir\"")
                {
                    break;
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            body_sender.send(request).unwrap();
        });

        let exporter = NetworkExporter::configured(true, Some(&endpoint));
        assert!(exporter.is_enabled());
        let telemetry = TelemetryState { exporter };
        let started = std::time::Instant::now();
        telemetry.record_tool_call("list_dir");
        assert!(started.elapsed() < Duration::from_millis(50));

        let request = body_receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains("POST /events/v1/metrics HTTP/1.1"));
        assert!(request.contains("\"resourceMetrics\""));
        assert!(request.contains("\"key\":\"tool.name\""));
        assert!(request.contains("\"stringValue\":\"list_dir\""));
        assert!(!request.contains("Authorization:"));
    }

    #[test]
    fn product_events_map_to_otlp_logs_without_local_identifiers() {
        let mut event = ProductEvent::new(ProductEventName::ActivationCompleted);
        event.id = "local-correlation-id".into();
        event.client_kind = Some("cursor".into());
        event.success = Some(true);
        let payload = otlp_payload(&ExportEvent::Product {
            event: Box::new(event),
        });
        let serialized = serde_json::to_string(&payload).unwrap();
        assert!(serialized.contains("\"resourceLogs\""));
        assert!(serialized.contains("\"stringValue\":\"activation_completed\""));
        assert!(serialized.contains("\"key\":\"client.kind\""));
        assert!(!serialized.contains("local-correlation-id"));
    }

    #[test]
    fn exporter_is_noop_without_consent_and_drops_unsafe_values() {
        let exporter = NetworkExporter::configured(false, Some("http://127.0.0.1:9/events"));
        assert!(!exporter.is_enabled());
        let telemetry = TelemetryState { exporter };
        telemetry.record_tool_call("/private/path");
        telemetry.record_error("secret-token");
        telemetry.record_latency("read_file", f64::NAN);
    }

    #[test]
    fn test_clear() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let store = ProductEventStore::new(Some(path.clone()));

        store
            .record(ProductEvent {
                id: "test-1".to_string(),
                timestamp: Utc::now().to_rfc3339(),
                name: ProductEventName::AppLaunched,
                schema_version: 1,
                app_version: "0.8.0".to_string(),
                os_arch: "linux-x86_64".to_string(),
                backend_type: None,
                workspace_template: None,
                access_profile: None,
                client_kind: None,
                success: None,
                failure_stage: None,
                error_code: None,
                duration_bucket: None,
            })
            .unwrap();

        store.clear().unwrap();
        let events = store.read_all().unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_diagnostics_bundle_no_secrets() {
        let bundle = DiagnosticsBundle {
            summary: DiagnosticsSummary {
                app_version: "0.8.0".to_string(),
                sidecar_version: Some("0.8.0".to_string()),
                sidecar_status: "healthy".to_string(),
                os_arch: "linux-x86_64".to_string(),
                keyring_status: "available".to_string(),
                config_file_status: "ok".to_string(),
                schema_versions: HashMap::new(),
                storage_count: 2,
                backend_counts: [("s3".into(), 1usize), ("local".into(), 1usize)]
                    .iter()
                    .cloned()
                    .collect(),
                exposed_storage_count: 1,
                enabled_tools: vec!["list_dir".into(), "read_file".into()],
                http_bind_category: "loopback".to_string(),
                port_available: true,
                last_error_codes: vec!["E001".into()],
                recent_audit_decision_count: 5,
            },
            sanitized_errors: vec![SanitizedError {
                stage: "sidecar".to_string(),
                error_code: "E001".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                count: 3,
            }],
            recent_product_events: vec![],
            recent_audit_events: vec![],
            redaction_manifest: RedactionManifest {
                redacted_fields: vec!["storage_name".into(), "path".into()],
                redacted_count: 12,
            },
            checksums: [("summary.json".into(), "abc123".into())]
                .iter()
                .cloned()
                .collect(),
        };

        let leaks = bundle.validate_no_secrets(&["secret-bucket", "my-storage"]);
        assert!(leaks.is_empty(), "secrets leaked: {leaks:?}");
    }
}
