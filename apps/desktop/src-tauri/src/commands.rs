#![allow(non_snake_case)]

use chrono::Utc;
use infimount_core::{operations, schema::StorageKindSchema, CoreError, Entry};
use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use infimount_mcp::server::ToolDefinition;
use infimount_mcp::settings::McpSettings;
use infimount_mcp::tools_storage::{
    export_config, import_config, validate_storage_record, ExportConfigInput, ExportConfigOutput,
    ImportConfigInput, ImportConfigOutput, ValidateStorageOutput,
};
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

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
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::write_full(&op, &path, &data).await
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

#[tauri::command]
pub async fn transfer_entries(
    state: State<'_, AppState>,
    fromSourceId: String,
    toSourceId: String,
    paths: Vec<String>,
    targetDir: String,
    operation: String,
    conflictPolicy: String,
) -> Result<(), CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;

    let op = match operation.as_str() {
        "copy" => operations::TransferOperation::Copy,
        "move" => operations::TransferOperation::Move,
        _ => {
            return Err(CoreError::Config(format!(
                "invalid transfer operation: {}",
                operation
            )));
        }
    };

    let policy = match conflictPolicy.as_str() {
        "fail" => operations::TransferConflictPolicy::Fail,
        "overwrite" => operations::TransferConflictPolicy::Overwrite,
        "skip" | "discard" => operations::TransferConflictPolicy::Skip,
        _ => {
            return Err(CoreError::Config(format!(
                "invalid transfer conflict policy: {}",
                conflictPolicy
            )));
        }
    };

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
