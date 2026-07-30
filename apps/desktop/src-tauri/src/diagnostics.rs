use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use infimount_core::secrets::SecretStore;
use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::settings::McpSettingsStore;
use infimount_mcp::telemetry::{
    build_os_arch, DiagnosticsBundle, DiagnosticsSummary, RedactionManifest,
};

use crate::app_settings::AppSettingsStore;

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticsExportResult {
    pub path: String,
    pub files: Vec<String>,
    pub checksums: HashMap<String, String>,
}

pub fn build_diagnostics_bundle(
    _settings_store: &AppSettingsStore,
    storage_registry: &StorageRegistry,
    settings: &McpSettingsStore,
    secret_store: &dyn SecretStore,
    _tool_count: usize,
    error_codes: Vec<String>,
    http_running: bool,
) -> Result<DiagnosticsBundle, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = build_os_arch();

    let keyring_status = match secret_store.status() {
        infimount_core::secrets::SecretStoreStatus::Available => "available".into(),
        infimount_core::secrets::SecretStoreStatus::Locked => "locked".into(),
        infimount_core::secrets::SecretStoreStatus::Unavailable { reason } => {
            format!("unavailable: {reason}")
        }
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

    let config_dir = infimount_mcp::registry::default_config_dir();
    let config_file_exists = config_dir.join("registry.json").exists();
    let config_file_status = if config_file_exists {
        "present".to_string()
    } else {
        "missing".to_string()
    };

    let mut schema_versions = HashMap::new();
    schema_versions.insert("workspaces".into(), 1u32);
    schema_versions.insert("backup".into(), 2u32);

    let http_bind_category = if http_running {
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

    let port_available = mcp_settings
        .as_ref()
        .map(|s| {
            if s.port == 0 {
                true
            } else {
                std::net::TcpListener::bind(format!("{}:{}", s.bind_address, s.port)).is_ok()
            }
        })
        .unwrap_or(true);

    let sidecar_version = check_sidecar_version();

    let summary = DiagnosticsSummary {
        app_version,
        sidecar_version,
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
        last_error_codes: error_codes,
        recent_audit_decision_count: 0,
    };

    let bundle = DiagnosticsBundle {
        summary,
        sanitized_errors: Vec::new(),
        redaction_manifest: RedactionManifest {
            redacted_fields: vec![
                "storage_name".into(),
                "path".into(),
                "bucket".into(),
                "endpoint".into(),
                "container".into(),
                "config_json".into(),
            ],
            redacted_count: 0,
        },
        checksums: HashMap::new(),
    };

    Ok(bundle)
}

fn check_sidecar_version() -> Option<String> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("mcp")))
        .filter(|p| p.exists())
        .or_else(|| {
            let path = infimount_mcp::registry::default_config_dir().join("mcp");
            if path.exists() { Some(path) } else { None }
        })?;

    let output = std::process::Command::new(&bundled)
        .arg("--version")
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn write_json_file(base: &PathBuf, filename: &str, json: &str) -> Result<(), String> {
    let path = base.join(filename);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(json.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn export_diagnostics_bundle(
    settings_store: &AppSettingsStore,
    storage_registry: &StorageRegistry,
    settings: &McpSettingsStore,
    secret_store: &dyn SecretStore,
    tool_count: usize,
    error_codes: Vec<String>,
    http_running: bool,
) -> Result<DiagnosticsExportResult, String> {
    let bundle = build_diagnostics_bundle(
        settings_store,
        storage_registry,
        settings,
        secret_store,
        tool_count,
        error_codes,
        http_running,
    )?;

    let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
    let dir_name = format!("diagnostics-{timestamp}");

    let mut base = dirs_data_dir();
    base.push(&dir_name);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;

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
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&checksums_path)
        .map_err(|e| e.to_string())?;
    for (name, hash) in &checksums {
        writeln!(&mut file, "{hash}  {name}").map_err(|e| e.to_string())?;
    }

    let bytes = fs::read(&checksums_path).map_err(|e| e.to_string())?;
    let hash = Sha256::digest(&bytes);
    checksums.insert("checksums.txt".to_string(), format!("{hash:x}"));

    let files: Vec<String> = vec![
        "summary.json".into(),
        "sanitized-errors.json".into(),
        "redaction-manifest.json".into(),
        "checksums.txt".into(),
    ];

    Ok(DiagnosticsExportResult {
        path: base.to_string_lossy().to_string(),
        files,
        checksums,
    })
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
    use std::sync::Arc;
    use tempfile::tempdir;

    fn dummy_bundle(
        registry: &StorageRegistry,
        settings_store: &McpSettingsStore,
    ) -> Result<DiagnosticsBundle, String> {
        build_diagnostics_bundle(
            &AppSettingsStore::new(None),
            registry,
            settings_store,
            &infimount_core::secrets::MemorySecretStore::new(),
            0,
            vec![],
            false,
        )
    }

    #[test]
    fn test_bundle_structure() {
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
    fn test_bundle_no_secrets() {
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
        let leaks = bundle.validate_no_secrets(&["secret-token", "my-bucket"]);
        assert!(leaks.is_empty(), "secrets leaked: {leaks:?}");
        std::env::remove_var("INFIMOUNT_DATA_DIR");
    }
}
