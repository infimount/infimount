#![allow(non_snake_case)]

use chrono::Utc;
use infimount_core::{operations, schema::StorageKindSchema, CoreError, Entry};
use infimount_mcp::audit::{mask_presigned_url, AuditDecision, AuditEvent, AuditStore};
use infimount_mcp::confirmation::PendingConfirmation;
use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::opendal_adapter::{get_capabilities, StorageBackendCapabilities};
use infimount_mcp::policy::McpStoragePolicy;
use infimount_mcp::registry::{
    default_config_dir, ensure_unique_name, validate_storage_name, StorageRecord,
};
use infimount_mcp::server::ToolDefinition;
use infimount_mcp::session::Session;
use infimount_mcp::settings::McpSettings;
use infimount_mcp::tools_storage::{
    export_config, import_config, validate_storage_record, ExportConfigInput, ExportConfigOutput,
    ImportConfigInput, ImportConfigOutput, ValidateStorageOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use crate::app_settings::AppSettings;
use crate::state::{AppState, McpClientSnippets, McpRuntimeStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDraft {
    pub name: String,
    pub backend: String,
    pub config: Value,
    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStoragesRequest {
    pub json: String,
    pub mode: String,
    pub on_conflict: String,
}

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries(&op, &path).await
}

#[tauri::command]
pub async fn list_entries_recursive(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries_recursive(&op, &path).await
}

#[tauri::command]
pub async fn stat_entry(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Entry, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::stat_entry(&op, &path).await
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::read_full(&op, &path).await
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    data: Vec<u8>,
    userMetadata: Option<HashMap<String, String>>,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::write_full_with_user_metadata(&op, &path, &data, userMetadata).await
}

#[tauri::command]
pub async fn create_directory(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::create_directory(&op, &path).await
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete(&op, &path).await
}

#[tauri::command]
pub async fn upload_dropped_files(
    state: State<'_, AppState>,
    sourceId: String,
    paths: Vec<String>,
    targetDir: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::upload_files_from_paths(&op, paths, targetDir).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgressPayload {
    job_id: String,
    completed_items: u64,
    total_items: u64,
    bytes_transferred: u64,
    total_bytes: u64,
    current_path: String,
}

fn parse_transfer_operation(operation: &str) -> Result<operations::TransferOperation, CoreError> {
    match operation {
        "copy" => Ok(operations::TransferOperation::Copy),
        "move" => Ok(operations::TransferOperation::Move),
        _ => Err(CoreError::Config(format!(
            "invalid transfer operation: {}",
            operation
        ))),
    }
}

fn parse_transfer_conflict_policy(
    conflict_policy: &str,
) -> Result<operations::TransferConflictPolicy, CoreError> {
    match conflict_policy {
        "fail" => Ok(operations::TransferConflictPolicy::Fail),
        "overwrite" => Ok(operations::TransferConflictPolicy::Overwrite),
        "skip" | "discard" => Ok(operations::TransferConflictPolicy::Skip),
        "rename" | "keep_both" => Ok(operations::TransferConflictPolicy::Rename),
        _ => Err(CoreError::Config(format!(
            "invalid transfer conflict policy: {}",
            conflict_policy
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn plan_transfer_entries(
    state: State<'_, AppState>,
    fromSourceId: String,
    toSourceId: String,
    paths: Vec<String>,
    targetDir: String,
    operation: String,
    conflictPolicy: String,
) -> Result<operations::TransferPlan, CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;
    operations::plan_transfer_entries(
        &from_op,
        &to_op,
        paths,
        &targetDir,
        parse_transfer_operation(&operation)?,
        fromSourceId == toSourceId,
        parse_transfer_conflict_policy(&conflictPolicy)?,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn transfer_entries(
    app: AppHandle,
    state: State<'_, AppState>,
    fromSourceId: String,
    toSourceId: String,
    paths: Vec<String>,
    targetDir: String,
    operation: String,
    conflictPolicy: String,
    jobId: Option<String>,
) -> Result<(), CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;

    let op = parse_transfer_operation(&operation)?;
    let policy = parse_transfer_conflict_policy(&conflictPolicy)?;

    if let Some(job_id) = jobId {
        state.clear_transfer_cancel(&job_id);
        let emit_app = app.clone();
        let emit_job_id = job_id.clone();
        let cancel_job_id = job_id.clone();
        let result = operations::transfer_entries_with_progress(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            op,
            fromSourceId == toSourceId,
            policy,
            move |progress| {
                let _ = emit_app.emit(
                    "infimount://transfer-progress",
                    TransferProgressPayload {
                        job_id: emit_job_id.clone(),
                        completed_items: progress.completed_items,
                        total_items: progress.total_items,
                        bytes_transferred: progress.bytes_transferred,
                        total_bytes: progress.total_bytes,
                        current_path: progress.current_path,
                    },
                );
            },
            || state.is_transfer_cancelled(&cancel_job_id),
        )
        .await;
        state.clear_transfer_cancel(&job_id);
        result
    } else {
        operations::transfer_entries(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            op,
            fromSourceId == toSourceId,
            policy,
        )
        .await
    }
}

#[tauri::command]
pub fn cancel_transfer_job(state: State<'_, AppState>, jobId: String) {
    state.request_transfer_cancel(&jobId);
}

#[tauri::command]
pub fn list_storages(state: State<'_, AppState>) -> Result<Vec<StorageRecord>, McpError> {
    state.list_storages()
}

#[tauri::command]
pub fn add_storage(
    state: State<'_, AppState>,
    storage: StorageDraft,
) -> Result<StorageRecord, McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let record = state.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &name, None)?;
        let mut record = StorageRecord::new(name.clone(), storage.backend.clone(), storage.config);
        record.enabled = storage.enabled;
        record.mcp_exposed = storage.mcp_exposed;
        record.read_only = storage.read_only;
        storages.push(record.clone());
        Ok(record)
    })?;
    Ok(record)
}

#[tauri::command]
pub fn update_storage(
    state: State<'_, AppState>,
    storageId: String,
    storage: StorageDraft,
) -> Result<StorageRecord, McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    state.registry.with_locked_mutation(|storages| {
        let idx = storages
            .iter()
            .position(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{}' not found", storageId),
                    serde_json::json!({ "storage_id": storageId }),
                )
            })?;

        ensure_unique_name(storages, &name, Some(storageId.as_str()))?;
        let mut updated = storages[idx].clone();
        updated.name = name;
        updated.backend = storage.backend;
        updated.config = storage.config;
        updated.enabled = storage.enabled;
        updated.mcp_exposed = storage.mcp_exposed;
        updated.read_only = storage.read_only;
        updated.updated_at = Utc::now().to_rfc3339();
        storages[idx] = updated.clone();
        Ok(updated)
    })
}

#[tauri::command]
pub fn remove_storage(state: State<'_, AppState>, storageId: String) -> Result<(), McpError> {
    state.registry.with_locked_mutation(|storages| {
        let original_len = storages.len();
        storages.retain(|storage| storage.id != storageId);
        if storages.len() == original_len {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("storage '{}' not found", storageId),
                serde_json::json!({ "storage_id": storageId }),
            ));
        }
        Ok(())
    })
}

#[tauri::command]
pub fn update_mcp_storage_policy(
    state: State<'_, AppState>,
    storageId: String,
    policy: McpStoragePolicy,
) -> Result<StorageRecord, McpError> {
    state.registry.with_locked_mutation(|storages| {
        let storage = storages
            .iter_mut()
            .find(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{}' not found", storageId),
                    serde_json::json!({ "storage_id": storageId }),
                )
            })?;

        storage.mcp_policy = normalize_mcp_policy(policy);
        storage.updated_at = Utc::now().to_rfc3339();
        Ok(storage.clone())
    })
}

fn normalize_mcp_policy(mut policy: McpStoragePolicy) -> McpStoragePolicy {
    policy.allowed_paths = normalize_policy_prefixes(policy.allowed_paths);
    policy.denied_paths = normalize_policy_prefixes(policy.denied_paths);
    policy
}

fn normalize_policy_prefixes(prefixes: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    prefixes
        .into_iter()
        .map(|prefix| normalize_policy_prefix(&prefix))
        .filter(|prefix| !prefix.is_empty())
        .filter(|prefix| seen.insert(prefix.clone()))
        .collect()
}

fn normalize_policy_prefix(prefix: &str) -> String {
    let mut segments = Vec::new();
    let normalized = prefix.trim().replace('\\', "/");
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value.to_string()),
        }
    }
    segments.join("/")
}

#[tauri::command]
pub async fn verify_storage(storage: StorageDraft) -> Result<ValidateStorageOutput, McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let mut record = StorageRecord::new(name, storage.backend, storage.config);
    record.enabled = storage.enabled;
    record.mcp_exposed = storage.mcp_exposed;
    record.read_only = storage.read_only;
    validate_storage_record(&record).await
}

#[tauri::command]
pub async fn import_storage_config(
    state: State<'_, AppState>,
    request: ImportStoragesRequest,
) -> Result<ImportConfigOutput, McpError> {
    import_config(
        &state.fs_context(),
        ImportConfigInput {
            json: request.json,
            mode: request.mode,
            on_conflict: request.on_conflict,
        },
    )
    .await
}

#[tauri::command]
pub async fn export_storage_config(
    state: State<'_, AppState>,
    includeSecrets: bool,
) -> Result<ExportConfigOutput, McpError> {
    export_config(
        &state.fs_context(),
        ExportConfigInput {
            include_secrets: includeSecrets,
        },
    )
    .await
}

#[tauri::command]
pub fn list_storage_schemas() -> Result<Vec<StorageKindSchema>, CoreError> {
    infimount_core::schema::list_storage_schemas()
}

#[tauri::command]
pub fn get_storage_capabilities(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<StorageBackendCapabilities, CoreError> {
    let op = state.operator_for_storage_id(&storageId)?;
    Ok(get_capabilities(&op))
}

#[tauri::command]
pub async fn generate_download_link(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    expiresSeconds: u64,
) -> Result<String, CoreError> {
    if !(60..=86_400).contains(&expiresSeconds) {
        return Err(CoreError::Config(
            "expiresSeconds must be between 60 and 86400".to_string(),
        ));
    }

    let op = state.operator_for_storage_id(&sourceId)?;
    let caps = op.info().full_capability();
    if !caps.presign_read {
        return Err(CoreError::Config(
            "storage backend does not support presigned download links".to_string(),
        ));
    }

    let metadata = op.stat(&path).await?;
    if metadata.is_dir() {
        return Err(CoreError::Config(
            "download links can only be created for files".to_string(),
        ));
    }

    let presigned = op
        .presign_read(&path, Duration::from_secs(expiresSeconds))
        .await?;
    Ok(presigned.uri().to_string())
}

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

#[tauri::command]
pub fn list_mcp_audit_events(limit: Option<usize>) -> Result<Vec<AuditEvent>, McpError> {
    AuditStore::new(None).list_recent(limit.unwrap_or(200).min(1000))
}

#[tauri::command]
pub fn clear_mcp_audit_events() -> Result<(), McpError> {
    AuditStore::new(None).clear()
}

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
    fs::create_dir_all(&export_dir).map_err(|error| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to create MCP audit export directory",
            serde_json::json!({ "io_error": error.to_string(), "path": export_dir }),
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
    fs::write(&path, payload).map_err(|error| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to write MCP audit export bundle",
            serde_json::json!({ "io_error": error.to_string(), "path": path }),
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

#[tauri::command]
pub fn get_mcp_settings(state: State<'_, AppState>) -> Result<McpSettings, McpError> {
    state.settings_store.load()
}

#[tauri::command]
pub fn list_mcp_tools() -> Vec<ToolDefinition> {
    infimount_mcp::server::tool_definitions()
}

#[tauri::command]
pub async fn update_mcp_settings(
    state: State<'_, AppState>,
    settings: McpSettings,
) -> Result<McpRuntimeStatus, McpError> {
    state.apply_mcp_settings(settings).await
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
pub async fn list_versions(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    let result =
        operations::list_file_versions(&op, &path, limit.unwrap_or(100), cursor.as_deref()).await?;
    Ok(serde_json::to_value(result).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn read_file_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::read_file_version(&op, &path, &version).await
}

#[tauri::command]
pub async fn delete_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete_file_version(&op, &path, &version).await?;
    Ok(serde_json::json!({ "deleted": true, "path": path, "version": version }))
}

fn validate_storage_draft(storage: &StorageDraft) -> McpResult<()> {
    if !storage.config.is_object() {
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "storage config must be a JSON object",
            serde_json::json!({}),
        ));
    }

    if !matches!(
        storage.backend.as_str(),
        "local" | "s3" | "azure_blob" | "webdav" | "gcs"
    ) {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{}'", storage.backend),
            serde_json::json!({ "backend": storage.backend }),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_policy_prefixes_deduplicates_and_collapses_segments() {
        let prefixes = normalize_policy_prefixes(vec![
            " /docs/ ".to_string(),
            "docs".to_string(),
            "./shared/".to_string(),
            "shared/tmp/../public".to_string(),
            "".to_string(),
        ]);

        assert_eq!(prefixes, vec!["docs", "shared", "shared/public"]);
    }
}
