use infimount_mcp::errors::McpError;
use serde::Deserialize;
use tauri::State;

use crate::app_settings::AppSettings;
use crate::state::AppState;

#[tauri::command]
pub fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    state.app_settings_store.load()
}

#[tauri::command]
pub fn complete_onboarding(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    state.app_settings_store.mark_onboarding_completed()
}

#[tauri::command]
pub fn skip_onboarding(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    state.app_settings_store.mark_onboarding_skipped()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWizardStateRequest {
    pub step: Option<String>,
    pub completed_steps: Vec<String>,
}

#[tauri::command]
pub fn save_wizard_state(
    state: State<'_, AppState>,
    request: SaveWizardStateRequest,
) -> Result<AppSettings, McpError> {
    state
        .app_settings_store
        .save_wizard_state(request.step.as_deref(), &request.completed_steps)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTelemetryConsentRequest {
    pub consent: bool,
}

#[tauri::command]
pub fn set_telemetry_consent(
    state: State<'_, AppState>,
    request: SetTelemetryConsentRequest,
) -> Result<AppSettings, McpError> {
    state.app_settings_store.set_telemetry_consent(request.consent)
}
