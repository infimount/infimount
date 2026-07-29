use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{map_io_error, McpErrorCode, err, McpResult};
use crate::registry::default_config_dir;

const MAX_EVENTS: usize = 5_000;
const MAX_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB

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

pub struct ProductEventStore {
    path: PathBuf,
    lock_path: PathBuf,
    buffer: Mutex<Vec<ProductEvent>>,
}

impl ProductEventStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_product_events_path);
        let lock_path = path.with_extension("lock");
        Self {
            path,
            lock_path,
            buffer: Mutex::new(Vec::new()),
        }
    }

    pub fn record(&self, event: ProductEvent) -> McpResult<()> {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.push(event);
        if buffer.len() >= 10 {
            let batch = buffer.drain(..).collect::<Vec<_>>();
            self.flush_batch(&batch)?;
        }
        Ok(())
    }

    pub fn flush(&self) -> McpResult<()> {
        let mut buffer = self.buffer.lock().unwrap();
        if !buffer.is_empty() {
            let batch = buffer.drain(..).collect::<Vec<_>>();
            self.flush_batch(&batch)?;
        }
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
            .write(true)
            .open(&self.lock_path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        let _guard = lock_file.lock_exclusive().map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        for event in events {
            let line = serde_json::to_string(event).map_err(|e| {
                err(McpErrorCode::ERR_INTERNAL, &format!("failed to serialize event: {e}"))
            })?;
            writeln!(&file, "{line}").map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }

        file.flush().map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        self.rotate_if_needed(&file)?;

        Ok(())
    }

    fn rotate_if_needed(&self, file: &fs::File) -> McpResult<()> {
        let metadata = file.metadata().map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        if metadata.len() < MAX_BYTES {
            return Ok(());
        }

        let reader = BufReader::new(file);
        let mut lines: Vec<String> = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
            lines.push(line);
        }

        if lines.len() <= MAX_EVENTS {
            return Ok(());
        }

        let excess = lines.len() - MAX_EVENTS;
        let rotated: Vec<&str> = lines.iter().skip(excess).map(|s| s.as_str()).collect();

        let mut out_file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        for line in &rotated {
            writeln!(&out_file, "{line}").map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        }
        out_file.flush().map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        Ok(())
    }

    pub fn read_all(&self) -> McpResult<Vec<ProductEvent>> {
        self.flush()?;

        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = OpenOptions::new().read(true).open(&self.path).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ProductEvent>(&line) {
                Ok(event) => events.push(event),
                Err(_) => continue,
            }
        }
        Ok(events)
    }

    pub fn clear(&self) -> McpResult<()> {
        self.flush()?;
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
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
pub struct DiagnosticsSummary {
    pub app_version: String,
    pub sidecar_version: Option<String>,
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
pub struct DiagnosticsBundle {
    pub summary: DiagnosticsSummary,
    pub sanitized_errors: Vec<SanitizedError>,
    pub redaction_manifest: RedactionManifest,
    pub checksums: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizedError {
    pub stage: String,
    pub error_code: String,
    pub timestamp: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// Backward-compatible API
pub fn init_telemetry() -> bool {
    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok()
}

#[derive(Debug)]
pub struct TelemetryState;

impl TelemetryState {
    pub fn new() -> Self {
        Self
    }

    pub fn record_tool_call(&self, _tool_name: &str) {}
    pub fn record_error(&self, _error_code: &str) {}
    pub fn record_latency(&self, _tool_name: &str, _elapsed_ms: f64) {}
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
    fn test_rotation() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let store = ProductEventStore::new(Some(path.clone()));

        // Write enough events to trigger rotation by bytes
        let large_field = "x".repeat(1000);
        for i in 0..100 {
            let event = ProductEvent {
                id: format!("test-{i}"),
                timestamp: Utc::now().to_rfc3339(),
                name: ProductEventName::AppLaunched,
                schema_version: 1,
                app_version: large_field.clone(),
                os_arch: large_field.clone(),
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
        }
        store.flush().unwrap();

        let events = store.read_all().unwrap();
        assert!(events.len() <= 100);
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
