use tauri::State;

use infimount_mcp::errors::McpError;
use infimount_mcp::telemetry::{build_os_arch, ProductEvent};

use crate::diagnostics::{export_diagnostics_bundle, DiagnosticsExportResult};
use crate::state::AppState;

#[tauri::command]
pub async fn export_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsExportResult, String> {
    let tool_count = state
        .settings_store
        .load()
        .ok()
        .map(|s| s.enabled_tools.len())
        .unwrap_or(0);

    let http_running = state.is_http_running().await;

    let error_codes = state
        .product_events
        .read_all()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.error_code)
        .collect();

    export_diagnostics_bundle(
        &state.app_settings_store,
        &state.registry,
        &state.settings_store,
        state.secret_store.as_ref(),
        tool_count,
        error_codes,
        http_running,
    )
}

#[tauri::command]
pub fn get_product_events(state: State<'_, AppState>) -> Result<Vec<ProductEvent>, McpError> {
    state.product_events.read_all()
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
