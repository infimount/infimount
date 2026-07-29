use tauri::State;

use infimount_mcp::errors::McpError;
use infimount_mcp::telemetry::{build_os_arch, ProductEvent};

use crate::diagnostics::{export_diagnostics_bundle, DiagnosticsExportResult};
use crate::state::AppState;

#[tauri::command]
pub fn export_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsExportResult, String> {
    let mcp_status = get_mcp_running_status(&state);
    export_diagnostics_bundle(
        &state.app_settings_store,
        mcp_status.tool_count,
        mcp_status.http_running,
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

struct McpRunningStatus {
    tool_count: usize,
    http_running: bool,
}

fn get_mcp_running_status(state: &AppState) -> McpRunningStatus {
    let status = state.settings_store.load().ok();

    McpRunningStatus {
        tool_count: status
            .as_ref()
            .map(|s| s.enabled_tools.len())
            .unwrap_or(0),
        http_running: false,
    }
}
