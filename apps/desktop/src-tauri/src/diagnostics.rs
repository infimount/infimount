use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    _tool_count: usize,
    http_running: bool,
) -> Result<DiagnosticsBundle, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let os_arch = build_os_arch();

    let keyring_status = if cfg!(target_os = "linux") {
        "check-keyring".to_string()
    } else {
        "available".to_string()
    };

    let http_bind_category = if http_running {
        "loopback".to_string()
    } else {
        "inactive".to_string()
    };

    let summary = DiagnosticsSummary {
        app_version,
        sidecar_version: None,
        os_arch,
        keyring_status,
        config_file_status: "check".to_string(),
        schema_versions: HashMap::new(),
        storage_count: 0,
        backend_counts: HashMap::new(),
        exposed_storage_count: 0,
        enabled_tools: Vec::new(),
        http_bind_category,
        port_available: true,
        last_error_codes: Vec::new(),
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
    tool_count: usize,
    http_running: bool,
) -> Result<DiagnosticsExportResult, String> {
    let bundle = build_diagnostics_bundle(settings_store, tool_count, http_running)?;
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

    // Add checksums.txt to the map after writing it
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
    use tempfile::tempdir;

    #[test]
    fn test_bundle_structure() {
        let dir = tempdir().unwrap();
        let mut settings_path = dir.path().to_path_buf();
        settings_path.push("settings.json");

        std::env::set_var(
            "INFIMOUNT_DATA_DIR",
            dir.path().to_string_lossy().to_string(),
        );

        let store = AppSettingsStore::new(Some(settings_path));
        store.reset_all().expect("init settings");

        let result = export_diagnostics_bundle(&store, 3, true).unwrap();
        assert!(result.path.contains("diagnostics-"));
        assert_eq!(result.files.len(), 4);
        assert!(result.checksums.contains_key("summary.json"));
        assert!(result.checksums.contains_key("checksums.txt"));
        assert!(result.path.contains("diagnostics-"));
    }

    #[test]
    fn test_bundle_no_secrets() {
        let dir = tempdir().unwrap();
        let mut settings_path = dir.path().to_path_buf();
        settings_path.push("settings.json");

        std::env::set_var(
            "INFIMOUNT_DATA_DIR",
            dir.path().to_string_lossy().to_string(),
        );

        let store = AppSettingsStore::new(Some(settings_path));
        store.reset_all().expect("init settings");

        let bundle = build_diagnostics_bundle(&store, 0, false).unwrap();
        let leaks = bundle.validate_no_secrets(&["secret-token", "my-bucket"]);
        assert!(leaks.is_empty(), "secrets leaked: {leaks:?}");
    }
}
