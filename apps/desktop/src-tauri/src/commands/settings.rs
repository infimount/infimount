#![allow(non_snake_case)]

use infimount_mcp::errors::McpError;
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
