use tauri::State;

use infimount_mcp::audit::AuditStore;
use infimount_mcp::errors::McpError;
use infimount_mcp::telemetry::{build_os_arch, ProductEvent};

use crate::diagnostics::{
    export_diagnostics_bundle, reveal_diagnostics_export as reveal_export, DiagnosticsExportResult,
    DiagnosticsInput,
};
use crate::state::{AppState, StartupHealth};

#[tauri::command]
pub fn get_startup_health(state: State<'_, AppState>) -> StartupHealth {
    state.startup_health()
}

#[tauri::command]
pub async fn export_diagnostics(
    state: State<'_, AppState>,
) -> Result<DiagnosticsExportResult, String> {
    let http_running = state.is_http_running().await;

    let product_events = state.product_events.read_all().unwrap_or_default();
    let error_codes = product_events
        .iter()
        .filter_map(|event| event.error_code.clone())
        .collect();
    let audit_events = AuditStore::new(None).list_recent(100).unwrap_or_default();
    let sidecar =
        tauri::async_runtime::spawn_blocking(crate::activation_probe::validate_sidecar_binary)
            .await
            .map_err(|_| "sidecar diagnostics failed".to_string())?;
    let sidecar_status = if sidecar.version_match && sidecar.doctor_healthy {
        "healthy"
    } else if !sidecar.binary_found {
        "not_found"
    } else if !sidecar.executable {
        "not_executable"
    } else if matches!(
        sidecar.error_code.as_deref(),
        Some(
            "ERR_SIDECAR_CHECKSUM_MISSING"
                | "ERR_SIDECAR_CHECKSUM_MISMATCH"
                | "ERR_SIDECAR_CHECKSUM_FAILED"
        )
    ) {
        "checksum_failed"
    } else if !sidecar.version_match {
        "version_mismatch"
    } else {
        "doctor_failed"
    }
    .to_string();

    export_diagnostics_bundle(DiagnosticsInput {
        settings_store: &state.app_settings_store,
        storage_registry: &state.registry,
        settings: &state.settings_store,
        secret_store: state.secret_store.as_ref(),
        error_codes,
        http_running,
        sidecar_version: sidecar.version,
        sidecar_status,
        product_events,
        audit_events,
    })
}

#[tauri::command]
pub fn get_product_events(state: State<'_, AppState>) -> Result<Vec<ProductEvent>, McpError> {
    state.product_events.read_all()
}

#[tauri::command]
pub fn clear_product_events(state: State<'_, AppState>) -> Result<(), McpError> {
    state.product_events.clear()
}

#[tauri::command]
pub fn reveal_diagnostics_export(export_id: String) -> Result<(), String> {
    reveal_export(&export_id)
}

#[tauri::command]
pub fn get_os_info() -> Result<OsInfo, McpError> {
    Ok(OsInfo {
        os_arch: build_os_arch(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub os_arch: String,
    pub app_version: String,
}
