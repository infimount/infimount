use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode};
use serde::Deserialize;
use tauri::State;

use crate::app_settings::AppSettings;
use crate::state::AppState;

#[tauri::command]
pub fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    state.app_settings_store.load()
}

fn require_current_probe(
    probe: &crate::activation_probe::ActivationProbeOutput,
) -> Result<(), McpError> {
    if probe.overall_ok {
        return Ok(());
    }
    Err(err_with_details(
        McpErrorCode::ERR_INTERNAL,
        "activation cannot complete until the current real MCP probe passes",
        serde_json::json!({
            "probeErrorCode": probe.error_code,
            "sidecarVersionMatch": probe.sidecar.version_match,
            "sidecarDoctorHealthy": probe.sidecar.doctor_healthy,
            "handshakeOk": probe.mcp_handshake_ok,
            "allowedOperationOk": probe.mcp_allowed_op_ok,
            "denialProven": probe.mcp_denial_proven,
        }),
    ))
}

#[tauri::command]
pub async fn complete_onboarding(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let probe = crate::activation_probe::run_activation_probe(
        state.registry.clone(),
        &state.product_events,
    )
    .await;
    require_current_probe(&probe)?;
    let result = state.app_settings_store.mark_onboarding_completed();
    if result.is_ok() {
        let mut event = infimount_mcp::telemetry::ProductEvent::new(
            infimount_mcp::telemetry::ProductEventName::ActivationCompleted,
        );
        event.success = Some(true);
        let _ = state.product_events.record(event);
    }
    result
}

#[tauri::command]
pub async fn skip_onboarding(state: State<'_, AppState>) -> Result<AppSettings, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    state.app_settings_store.mark_onboarding_skipped()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWizardStateRequest {
    pub step: Option<String>,
    pub completed_steps: Vec<String>,
}

#[tauri::command]
pub async fn save_wizard_state(
    state: State<'_, AppState>,
    request: SaveWizardStateRequest,
) -> Result<AppSettings, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let result = state
        .app_settings_store
        .save_wizard_state(request.step.as_deref(), &request.completed_steps);
    if result.is_ok() && !request.completed_steps.is_empty() {
        let mut event = infimount_mcp::telemetry::ProductEvent::new(
            infimount_mcp::telemetry::ProductEventName::OnboardingStepCompleted,
        );
        event.success = Some(true);
        let _ = state.product_events.record(event);
    }
    result
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTelemetryConsentRequest {
    pub consent: bool,
}

#[tauri::command]
pub async fn set_telemetry_consent(
    state: State<'_, AppState>,
    request: SetTelemetryConsentRequest,
) -> Result<AppSettings, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    state
        .app_settings_store
        .set_telemetry_consent(request.consent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_probe::{ActivationProbeOutput, SidecarValidation};

    fn probe(overall_ok: bool) -> ActivationProbeOutput {
        ActivationProbeOutput {
            sidecar: SidecarValidation {
                binary_found: true,
                executable: true,
                canonical_path: Some("/opt/infimount/mcp".to_string()),
                version: Some("0.8.0".to_string()),
                version_match: overall_ok,
                doctor_healthy: overall_ok,
                sha256: Some("a".repeat(64)),
                checksum_verified: overall_ok,
                error_code: (!overall_ok).then(|| "ERR_SIDECAR_VERSION_MISMATCH".to_string()),
            },
            mcp_handshake_ok: overall_ok,
            mcp_allowed_op_ok: overall_ok,
            mcp_denial_proven: overall_ok,
            mcp_audit_ok: overall_ok,
            overall_ok,
            error_code: (!overall_ok).then(|| "ERR_ACTIVATION_PROBE_FAILED".to_string()),
        }
    }

    #[test]
    fn onboarding_completion_requires_a_current_successful_probe() {
        assert!(require_current_probe(&probe(true)).is_ok());
        let error = require_current_probe(&probe(false)).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INTERNAL);
        assert_eq!(
            error.details["probeErrorCode"],
            "ERR_ACTIVATION_PROBE_FAILED"
        );
    }
}
