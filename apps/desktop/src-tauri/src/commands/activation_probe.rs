use infimount_mcp::errors::McpError;
use tauri::State;

use crate::activation_probe as probe_logic;
use crate::state::AppState;

#[tauri::command]
pub async fn run_activation_probe(
    state: State<'_, AppState>,
) -> Result<probe_logic::ActivationProbeOutput, McpError> {
    state.require_operational()?;
    Ok(probe_logic::run_activation_probe(state.registry.clone(), &state.product_events).await)
}
