#![allow(non_snake_case)]

use chrono::Utc;
use infimount_mcp::audit::{mask_presigned_url, AuditDecision, AuditEvent, AuditStore};
use infimount_mcp::confirmation::PendingConfirmation;
use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode};
use infimount_mcp::registry::default_config_dir;
use infimount_mcp::server::ToolDefinition;
use infimount_mcp::session::Session;
use infimount_mcp::settings::McpSettings;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::{AppState, AuthTokenMutation, McpClientSnippets, McpRuntimeStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditBundleRequest {
    pub events: Vec<AuditEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuditRedactionManifest {
    pub secrets_included: bool,
    pub file_contents_included: bool,
    pub auth_tokens_included: bool,
    pub presigned_url_query_strings: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditBundleOutput {
    pub path: String,
    pub event_count: usize,
    pub redaction_manifest: McpAuditRedactionManifest,
}

#[tauri::command]
pub fn list_mcp_tools() -> Vec<ToolDefinition> {
    infimount_mcp::server::tool_definitions()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettingsUpdate {
    pub enabled: bool,
    pub transport: infimount_mcp::settings::McpTransport,
    pub bind_address: String,
    pub port: u16,
    pub enabled_tools: Vec<String>,
    pub auth_token_mutation: AuthTokenMutation,
}

#[tauri::command]
pub async fn update_mcp_settings_with_auth(
    state: State<'_, AppState>,
    update: McpSettingsUpdate,
) -> Result<McpRuntimeStatus, McpError> {
    let settings = McpSettings {
        schema_version: infimount_mcp::settings::MCP_SETTINGS_SCHEMA_VERSION,
        enabled: update.enabled,
        transport: update.transport,
        bind_address: update.bind_address,
        port: update.port,
        enabled_tools: update.enabled_tools,
        auth_token_ref: None,
        auth_token: None,
        security_baseline_version: infimount_mcp::settings::SECURITY_BASELINE_VERSION,
    };

    state
        .apply_mcp_settings_with_auth(settings, update.auth_token_mutation)
        .await
}

#[tauri::command]
pub async fn get_mcp_status(state: State<'_, AppState>) -> Result<McpRuntimeStatus, McpError> {
    state.mcp_status().await
}

#[tauri::command]
pub async fn start_mcp_http(state: State<'_, AppState>) -> Result<McpRuntimeStatus, McpError> {
    state.start_http_server().await
}

#[tauri::command]
pub async fn stop_mcp_http(state: State<'_, AppState>) -> Result<McpRuntimeStatus, McpError> {
    state.stop_http_server().await
}

#[tauri::command]
pub async fn get_mcp_client_snippets(
    state: State<'_, AppState>,
) -> Result<McpClientSnippets, McpError> {
    state.client_snippets().await
}

#[tauri::command]
pub fn list_mcp_audit_events(limit: Option<usize>) -> Result<Vec<AuditEvent>, McpError> {
    AuditStore::new(None).list_recent(limit.unwrap_or(200).min(1000))
}

#[tauri::command]
pub fn clear_mcp_audit_events() -> Result<(), McpError> {
    AuditStore::new(None).clear()
}

#[tauri::command]
pub fn export_mcp_audit_bundle(
    request: ExportMcpAuditBundleRequest,
) -> Result<ExportMcpAuditBundleOutput, McpError> {
    let events = request
        .events
        .into_iter()
        .map(sanitize_audit_event_for_export)
        .collect::<Vec<_>>();
    let manifest = McpAuditRedactionManifest {
        secrets_included: false,
        file_contents_included: false,
        auth_tokens_included: false,
        presigned_url_query_strings: "redacted".to_string(),
    };
    let bundle = serde_json::json!({
        "generatedAt": Utc::now().to_rfc3339(),
        "eventCount": events.len(),
        "redactionManifest": manifest,
        "events": events,
    });

    let export_dir = default_config_dir().join("exports");
    infimount_core::atomic_file::create_dir_all(&export_dir).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to create MCP audit export directory",
            serde_json::json!({}),
        )
    })?;
    let filename = format!("mcp-audit-{}.json", Utc::now().format("%Y%m%dT%H%M%SZ"));
    let path = export_dir.join(filename);
    let payload = serde_json::to_vec_pretty(&bundle).map_err(|error| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize MCP audit export bundle",
            serde_json::json!({ "serde_error": error.to_string() }),
        )
    })?;
    infimount_core::atomic_file::atomic_write_file(
        &path,
        &payload,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to write MCP audit export bundle",
            serde_json::json!({}),
        )
    })?;

    Ok(ExportMcpAuditBundleOutput {
        path: path.display().to_string(),
        event_count: events.len(),
        redaction_manifest: manifest,
    })
}

fn sanitize_audit_event_for_export(mut event: AuditEvent) -> AuditEvent {
    event.path = event.path.map(|path| mask_presigned_url(&path));
    event
}

#[tauri::command]
pub async fn list_pending_mcp_confirmations(
    state: State<'_, AppState>,
) -> Result<Vec<PendingConfirmation>, McpError> {
    Ok(state.confirmations.list_pending().await)
}

#[tauri::command]
pub async fn list_active_mcp_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<Session>, McpError> {
    Ok(state.sessions.list_active().await)
}

#[tauri::command]
pub async fn approve_mcp_confirmation(
    state: State<'_, AppState>,
    operationId: String,
) -> Result<PendingConfirmation, McpError> {
    let pending = state.confirmations.approve(&operationId).await?;
    append_confirmation_audit(&pending, AuditDecision::Confirmed)?;
    Ok(pending)
}

#[tauri::command]
pub async fn deny_mcp_confirmation(
    state: State<'_, AppState>,
    operationId: String,
) -> Result<PendingConfirmation, McpError> {
    let pending = state.confirmations.deny(&operationId).await?;
    append_confirmation_audit(&pending, AuditDecision::Denied)?;
    Ok(pending)
}

fn append_confirmation_audit(
    pending: &PendingConfirmation,
    decision: AuditDecision,
) -> Result<(), McpError> {
    let mut event = AuditEvent::new(&pending.tool_name, pending.operation);
    event.actor_type = "desktop".to_string();
    event.storage_id = Some(pending.storage_id.clone());
    event.storage_name = Some(pending.storage_name.clone());
    event.path = Some(pending.path.clone());
    event.decision = decision;
    event.confirmation_id = Some(pending.operation_id.clone());
    AuditStore::new(None).append(event)
}
