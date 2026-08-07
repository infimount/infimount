use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use infimount_core::secrets::SecretStore;
use infimount_mcp::audit::AuditEvent;
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use infimount_mcp::settings::McpSettingsStore;
use infimount_mcp::telemetry::{
    build_os_arch, AuditEventSummary, DiagnosticsBundle, DiagnosticsSummary, ProductEvent,
    ProductEventSummary, RedactionManifest,
};

use crate::app_settings::AppSettingsStore;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExportResult {
    pub export_id: String,
    pub bundle_name: String,
    pub files: Vec<String>,
    pub checksums: HashMap<String, String>,
}

pub struct DiagnosticsInput<'a> {
    pub settings_store: &'a AppSettingsStore,
    pub storage_registry: &'a StorageRegistry,
    pub settings: &'a McpSettingsStore,
    pub secret_store: &'a dyn SecretStore,
    pub error_codes: Vec<String>,
    pub http_running: bool,
    pub sidecar_version: Option<String>,
    pub sidecar_status: String,
    pub product_events: Vec<ProductEvent>,
    pub audit_events: Vec<AuditEvent>,
}

pub fn build_diagnostics_bundle(input: &DiagnosticsInput<'_>) -> Result<DiagnosticsBundle, String> {
    let DiagnosticsInput {
        settings_store,
        storage_registry,
        settings,
        secret_store,
        error_codes,
        http_running,
        sidecar_version,
        sidecar_status,
        product_events,
        audit_events,
    } = input;
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = build_os_arch();

    let keyring_status = match secret_store.status() {
        infimount_core::secrets::SecretStoreStatus::Available => "available".into(),
        infimount_core::secrets::SecretStoreStatus::Locked => "locked".into(),
        infimount_core::secrets::SecretStoreStatus::Unavailable { .. } => "unavailable".into(),
    };

    let storages = storage_registry.load_all().map_err(|e| e.message)?;
    let storage_count = storages.len();
    let exposed_storage_count = storages.iter().filter(|s| s.mcp_exposed).count();
    let mut backend_counts: HashMap<String, usize> = HashMap::new();
    for s in &storages {
        *backend_counts.entry(s.backend.clone()).or_default() += 1;
    }

    let mcp_settings = settings.load().ok();
    let enabled_tools = mcp_settings
        .as_ref()
        .map(|s| s.enabled_tools.clone())
        .unwrap_or_default();

    let config_file_exists = storage_registry.path().exists();
    let config_file_status = if config_file_exists {
        "present".to_string()
    } else {
        "missing".to_string()
    };

    let mut schema_versions = HashMap::new();
    schema_versions.insert("workspaces".into(), 1u32);
    schema_versions.insert("backup".into(), 2u32);
    schema_versions.insert("app_settings".into(), 1u32);
    schema_versions.insert("mcp_policy".into(), 2u32);

    let http_bind_category = if *http_running {
        mcp_settings
            .as_ref()
            .map(|s| {
                if s.bind_address.starts_with("127.")
                    || s.bind_address.eq_ignore_ascii_case("localhost")
                    || s.bind_address == "::1"
                {
                    "loopback".to_string()
                } else {
                    "non-loopback".to_string()
                }
            })
            .unwrap_or_else(|| "unknown".into())
    } else {
        "inactive".to_string()
    };

    let port_available = if *http_running {
        true
    } else {
        mcp_settings
            .as_ref()
            .map(|s| {
                s.port == 0
                    || std::net::TcpListener::bind(format!("{}:{}", s.bind_address, s.port)).is_ok()
            })
            .unwrap_or(true)
    };

    let app_settings_status = settings_store.load().is_ok();
    let safe_error_codes = error_codes
        .iter()
        .filter(|code| is_safe_code(code))
        .cloned()
        .collect::<Vec<_>>();
    let mut error_counts = HashMap::<String, u32>::new();
    for code in &safe_error_codes {
        *error_counts.entry(code.clone()).or_default() += 1;
    }
    let sanitized_errors = error_counts
        .into_iter()
        .map(
            |(error_code, count)| infimount_mcp::telemetry::SanitizedError {
                stage: "product_event".into(),
                error_code,
                timestamp: Utc::now().to_rfc3339(),
                count,
            },
        )
        .collect();

    let recent_product_events = product_events
        .iter()
        .rev()
        .filter(|event| event.validate().is_ok())
        .take(100)
        .cloned()
        .map(|event| ProductEventSummary {
            name: event.name,
            success: event.success,
            failure_stage: event.failure_stage,
            error_code: event.error_code.filter(|code| is_safe_code(code)),
            duration_bucket: event.duration_bucket,
        })
        .collect::<Vec<_>>();
    let recent_audit_events = audit_events
        .iter()
        .take(100)
        .cloned()
        .filter_map(sanitize_audit_event)
        .collect::<Vec<_>>();

    let summary = DiagnosticsSummary {
        app_version,
        sidecar_version: sidecar_version.clone(),
        sidecar_status: sidecar_status.clone(),
        os_arch,
        keyring_status,
        config_file_status,
        schema_versions,
        storage_count,
        backend_counts,
        exposed_storage_count,
        enabled_tools,
        http_bind_category,
        port_available,
        last_error_codes: safe_error_codes,
        recent_audit_decision_count: recent_audit_events.len(),
    };

    let corpus = sensitive_corpus(&storages);
    let bundle = DiagnosticsBundle {
        summary,
        sanitized_errors,
        recent_product_events,
        recent_audit_events,
        redaction_manifest: RedactionManifest {
            redacted_fields: vec![
                "storage_name".into(),
                "path".into(),
                "bucket".into(),
                "endpoint".into(),
                "container".into(),
                "config_json".into(),
            ],
            redacted_count: corpus.len() + usize::from(app_settings_status),
        },
        checksums: HashMap::new(),
    };

    validate_bundle_against_corpus(&bundle, &corpus)?;
    Ok(bundle)
}

fn sanitize_audit_event(event: AuditEvent) -> Option<AuditEventSummary> {
    if !is_safe_identifier(&event.tool_name) {
        return None;
    }
    let operation = serde_json::to_value(event.operation)
        .ok()?
        .as_str()?
        .to_string();
    let decision = serde_json::to_value(event.decision)
        .ok()?
        .as_str()?
        .to_string();
    Some(AuditEventSummary {
        tool_name: event.tool_name,
        operation,
        decision,
        error_code: event.error_code.filter(|code| is_safe_code(code)),
        duration_bucket: event.duration_ms.map(|duration| {
            if duration < 100 {
                "fast"
            } else if duration < 1_000 {
                "moderate"
            } else {
                "slow"
            }
            .to_string()
        }),
    })
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_safe_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn sensitive_corpus(storages: &[StorageRecord]) -> Vec<String> {
    fn collect_strings(value: &serde_json::Value, output: &mut Vec<String>) {
        match value {
            serde_json::Value::String(value) if value.len() >= 4 => output.push(value.clone()),
            serde_json::Value::Array(values) => {
                for value in values {
                    collect_strings(value, output);
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    collect_strings(value, output);
                }
            }
            _ => {}
        }
    }

    let mut corpus = Vec::new();
    for storage in storages {
        if storage.name.len() >= 4 {
            corpus.push(storage.name.clone());
        }
        if let Some(secret_ref) = &storage.secret_ref {
            corpus.push(secret_ref.clone());
        }
        collect_strings(&storage.config, &mut corpus);
    }
    corpus.sort();
    corpus.dedup();
    corpus
}

fn validate_bundle_against_corpus(
    bundle: &DiagnosticsBundle,
    corpus: &[String],
) -> Result<(), String> {
    let serialized = serde_json::to_string(bundle).map_err(|_| "diagnostics validation failed")?;
    if corpus.iter().any(|seed| serialized.contains(seed)) {
        return Err("diagnostics redaction validation failed".into());
    }
    Ok(())
}

fn validate_export_files(base: &Path, corpus: &[String]) -> Result<(), String> {
    for entry in fs::read_dir(base).map_err(|_| "diagnostics validation failed")? {
        let path = entry.map_err(|_| "diagnostics validation failed")?.path();
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(path).map_err(|_| "diagnostics validation failed")?;
        if corpus.iter().any(|seed| {
            !seed.is_empty()
                && bytes
                    .windows(seed.len())
                    .any(|part| part == seed.as_bytes())
        }) {
            return Err("diagnostics redaction validation failed".into());
        }
    }
    Ok(())
}

fn write_json_file(base: &Path, filename: &str, json: &str) -> Result<(), String> {
    infimount_core::atomic_file::atomic_write_file(
        &base.join(filename),
        json.as_bytes(),
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| "failed to write diagnostics export".to_string())
}

pub fn export_diagnostics_bundle(
    input: DiagnosticsInput<'_>,
) -> Result<DiagnosticsExportResult, String> {
    let bundle = build_diagnostics_bundle(&input)?;
    let corpus = sensitive_corpus(&input.storage_registry.load_all().map_err(|e| e.message)?);

    let export_id = uuid::Uuid::new_v4().to_string();
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
    let dir_name = format!("diagnostics-{timestamp}-{export_id}");

    let mut base = dirs_data_dir();
    base.push(&dir_name);
    fs::create_dir_all(&base).map_err(|_| "failed to create diagnostics export".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700))
            .map_err(|_| "failed to secure diagnostics export".to_string())?;
    }

    write_json_file(
        &base,
        "summary.json",
        &serde_json::to_string_pretty(&bundle.summary).map_err(|e| e.to_string())?,
    )?;
    write_json_file(
        &base,
        "sanitized-errors.json",
        &serde_json::to_string_pretty(&bundle.sanitized_errors).map_err(|e| e.to_string())?,
    )?;
    write_json_file(
        &base,
        "product-events.json",
        &serde_json::to_string_pretty(&bundle.recent_product_events).map_err(|e| e.to_string())?,
    )?;
    write_json_file(
        &base,
        "mcp-audit-summary.json",
        &serde_json::to_string_pretty(&bundle.recent_audit_events).map_err(|e| e.to_string())?,
    )?;
    write_json_file(
        &base,
        "redaction-manifest.json",
        &serde_json::to_string_pretty(&bundle.redaction_manifest).map_err(|e| e.to_string())?,
    )?;

    let mut checksums: HashMap<String, String> = HashMap::new();
    for entry in fs::read_dir(&base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let hash = Sha256::digest(&bytes);
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            checksums.insert(filename, format!("{hash:x}"));
        }
    }

    let checksums_path = base.join("checksums.txt");
    let mut checksum_rows = checksums.iter().collect::<Vec<_>>();
    checksum_rows.sort_by_key(|(name, _)| *name);
    let checksum_text = checksum_rows
        .into_iter()
        .map(|(name, hash)| format!("{hash}  {name}\n"))
        .collect::<String>();
    infimount_core::atomic_file::atomic_write_file(
        &checksums_path,
        checksum_text.as_bytes(),
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| "failed to write diagnostics checksums".to_string())?;

    let bytes = fs::read(&checksums_path).map_err(|e| e.to_string())?;
    let hash = Sha256::digest(&bytes);
    checksums.insert("checksums.txt".to_string(), format!("{hash:x}"));

    let files: Vec<String> = vec![
        "summary.json".into(),
        "sanitized-errors.json".into(),
        "product-events.json".into(),
        "mcp-audit-summary.json".into(),
        "redaction-manifest.json".into(),
        "checksums.txt".into(),
    ];

    if let Err(error) = validate_export_files(&base, &corpus) {
        let _ = fs::remove_dir_all(&base);
        return Err(error);
    }

    Ok(DiagnosticsExportResult {
        export_id,
        bundle_name: dir_name,
        files,
        checksums,
    })
}

pub fn reveal_diagnostics_export(export_id: &str) -> Result<(), String> {
    if export_id.len() != 36
        || !export_id
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
    {
        return Err("invalid diagnostics export id".into());
    }
    let base = dirs_data_dir();
    let target = fs::read_dir(&base)
        .map_err(|_| "diagnostics export not found".to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(export_id))
        })
        .ok_or_else(|| "diagnostics export not found".to_string())?;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        command.arg(&target);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(&target);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&target);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|_| "failed to reveal diagnostics export".to_string())
}

fn dirs_data_dir() -> PathBuf {
    let mut path = data_dir_inner();
    path.push("diagnostics");
    path
}

fn data_dir_inner() -> PathBuf {
    if let Ok(val) = std::env::var("INFIMOUNT_DATA_DIR") {
        return PathBuf::from(val);
    }
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("infimount");
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    static DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

    fn dummy_bundle(
        registry: &StorageRegistry,
        settings_store: &McpSettingsStore,
    ) -> Result<DiagnosticsBundle, String> {
        build_diagnostics_bundle(&DiagnosticsInput {
            settings_store: &AppSettingsStore::new(None),
            storage_registry: registry,
            settings: settings_store,
            secret_store: &infimount_core::secrets::MemorySecretStore::new(),
            error_codes: vec![],
            http_running: false,
            sidecar_version: None,
            sidecar_status: "not_found".into(),
            product_events: vec![],
            audit_events: vec![],
        })
    }

    #[test]
    fn test_bundle_structure() {
        let _data_dir_guard = DATA_DIR_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        std::env::set_var(
            "INFIMOUNT_DATA_DIR",
            dir.path().to_string_lossy().to_string(),
        );

        let registry_dir = dir.path().join("config");
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let settings_store = McpSettingsStore::with_secret_store(
            Some(registry_dir.join("settings.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );

        let bundle = dummy_bundle(&registry, &settings_store).unwrap();
        assert_eq!(bundle.summary.storage_count, 0);
        assert_eq!(bundle.summary.http_bind_category, "inactive");
        assert!(bundle.summary.app_version.contains("0."));
        std::env::remove_var("INFIMOUNT_DATA_DIR");
    }

    #[test]
    fn bundle_summarizes_sidecar_events_and_audit_without_resource_metadata() {
        let _data_dir_guard = DATA_DIR_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let registry_dir = dir.path().join("config");
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let settings_store = McpSettingsStore::with_secret_store(
            Some(registry_dir.join("settings.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let product_event = ProductEvent {
            id: "event-1".into(),
            timestamp: Utc::now().to_rfc3339(),
            name: infimount_mcp::telemetry::ProductEventName::SidecarVerified,
            schema_version: 1,
            app_version: "0.8.0".into(),
            os_arch: "linux-x86_64".into(),
            backend_type: None,
            workspace_template: None,
            access_profile: None,
            client_kind: None,
            success: Some(true),
            failure_stage: None,
            error_code: None,
            duration_bucket: Some("fast".into()),
        };
        let mut audit = infimount_mcp::audit::AuditEvent::new(
            "read_file",
            infimount_mcp::policy::McpOperation::Read,
        );
        audit.storage_name = Some("never-export-this-storage".into());
        audit.path = Some("/never/export/this/path".into());
        audit.decision = infimount_mcp::audit::AuditDecision::Allowed;
        audit.duration_ms = Some(25);

        let bundle = build_diagnostics_bundle(&DiagnosticsInput {
            settings_store: &AppSettingsStore::new(None),
            storage_registry: &registry,
            settings: &settings_store,
            secret_store: &infimount_core::secrets::MemorySecretStore::new(),
            error_codes: vec![],
            http_running: false,
            sidecar_version: Some("0.8.0".into()),
            sidecar_status: "healthy".into(),
            product_events: vec![product_event],
            audit_events: vec![audit],
        })
        .unwrap();
        assert_eq!(bundle.summary.sidecar_version.as_deref(), Some("0.8.0"));
        assert_eq!(bundle.summary.sidecar_status, "healthy");
        assert_eq!(bundle.recent_product_events.len(), 1);
        assert_eq!(bundle.recent_audit_events.len(), 1);
        let serialized = serde_json::to_string(&bundle).unwrap();
        assert!(!serialized.contains("never-export-this-storage"));
        assert!(!serialized.contains("/never/export/this/path"));
    }

    #[test]
    fn generated_file_validation_rejects_seeded_corpus() {
        let _data_dir_guard = DATA_DIR_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("summary.json"),
            r#"{"value":"seeded-secret"}"#,
        )
        .unwrap();
        let error = validate_export_files(dir.path(), &["seeded-secret".into()]).unwrap_err();
        assert_eq!(error, "diagnostics redaction validation failed");
    }

    #[test]
    fn exported_bundle_is_bounded_and_passes_post_generation_corpus_validation() {
        let _data_dir_guard = DATA_DIR_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        std::env::set_var("INFIMOUNT_DATA_DIR", dir.path());
        let registry_dir = dir.path().join("config");
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let settings_store = McpSettingsStore::with_secret_store(
            Some(registry_dir.join("settings.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        registry
            .with_locked_mutation(|storages| {
                storages.push(infimount_mcp::registry::StorageRecord::new(
                    "private-export-seed".into(),
                    "s3".into(),
                    serde_json::json!({
                        "bucket": "private-bucket-seed",
                        "endpoint": "https://private-endpoint.invalid"
                    }),
                ));
                Ok(())
            })
            .unwrap();
        let events = (0..150)
            .map(|_| ProductEvent::new(infimount_mcp::telemetry::ProductEventName::AppLaunched))
            .collect();

        let result = export_diagnostics_bundle(DiagnosticsInput {
            settings_store: &AppSettingsStore::new(None),
            storage_registry: &registry,
            settings: &settings_store,
            secret_store: &infimount_core::secrets::MemorySecretStore::new(),
            error_codes: vec![],
            http_running: false,
            sidecar_version: Some("0.8.0".into()),
            sidecar_status: "healthy".into(),
            product_events: events,
            audit_events: vec![],
        })
        .unwrap();

        assert_eq!(result.files.len(), 6);
        let export_dir = dirs_data_dir().join(result.bundle_name);
        let product_events: Vec<ProductEventSummary> =
            serde_json::from_slice(&fs::read(export_dir.join("product-events.json")).unwrap())
                .unwrap();
        assert_eq!(product_events.len(), 100);
        let corpus = [
            "private-export-seed",
            "private-bucket-seed",
            "private-endpoint.invalid",
        ];
        for entry in fs::read_dir(export_dir).unwrap() {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(corpus.iter().all(|seed| !text.contains(seed)));
        }
        std::env::remove_var("INFIMOUNT_DATA_DIR");
    }

    #[test]
    fn test_bundle_no_secrets() {
        let _data_dir_guard = DATA_DIR_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        std::env::set_var(
            "INFIMOUNT_DATA_DIR",
            dir.path().to_string_lossy().to_string(),
        );

        let registry_dir = dir.path().join("config");
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let settings_store = McpSettingsStore::with_secret_store(
            Some(registry_dir.join("settings.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );

        registry
            .with_locked_mutation(|storages| {
                let mut storage = infimount_mcp::registry::StorageRecord::new(
                    "seeded-private-storage".into(),
                    "s3".into(),
                    serde_json::json!({
                        "bucket": "my-bucket",
                        "endpoint": "https://user:secret-token@example.invalid",
                        "root": "/private/customer/path"
                    }),
                );
                storage.secret_ref = Some("storage/seeded-secret".into());
                storages.push(storage);
                Ok(())
            })
            .unwrap();

        let bundle = dummy_bundle(&registry, &settings_store).unwrap();
        let leaks = bundle.validate_no_secrets(&[
            "secret-token",
            "my-bucket",
            "seeded-private-storage",
            "/private/customer/path",
            "example.invalid",
            "storage/seeded-secret",
        ]);
        assert!(leaks.is_empty(), "secrets leaked: {leaks:?}");
        std::env::remove_var("INFIMOUNT_DATA_DIR");
    }
}
