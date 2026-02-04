use tauri::State;

use crate::state::AppState;
use openhsb_core::{operations, schema::StorageKindSchema, CoreError, Entry, Source};

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
    operations::list_entries(&op, &path).await
}

#[tauri::command]
pub async fn stat_entry(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Entry, CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
    operations::stat_entry(&op, &path).await
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
    operations::read_full(&op, &path).await
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    data: Vec<u8>,
) -> Result<(), CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
    operations::write_full(&op, &path, &data).await
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
    operations::delete(&op, &path).await
}

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> Result<Vec<Source>, CoreError> {
    Ok(state.registry.list_sources().await)
}

#[tauri::command]
pub async fn replace_sources(
    state: State<'_, AppState>,
    sources: Vec<Source>,
) -> Result<(), CoreError> {
    state.registry.replace_sources(sources).await
}

#[tauri::command]
pub async fn add_source(state: State<'_, AppState>, source: Source) -> Result<(), CoreError> {
    state.registry.add_source(source).await
}

#[tauri::command]
pub async fn remove_source(
    state: State<'_, AppState>,
    sourceId: String,
) -> Result<(), CoreError> {
    state.registry.remove_source(&sourceId).await
}

#[tauri::command]
pub async fn update_source(state: State<'_, AppState>, source: Source) -> Result<(), CoreError> {
    state.registry.update_source(source).await
}

#[tauri::command]
pub async fn upload_dropped_files(
    state: State<'_, AppState>,
    sourceId: String,
    paths: Vec<String>,
    targetDir: String,
) -> Result<(), CoreError> {
    let op = state.registry.get_operator(&sourceId).await?;
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
    let from_op = state.registry.get_operator(&fromSourceId).await?;
    let to_op = state.registry.get_operator(&toSourceId).await?;

    let op = match operation.as_str() {
        "copy" => operations::TransferOperation::Copy,
        "move" => operations::TransferOperation::Move,
        _ => {
            return Err(CoreError::Config(format!(
                "invalid transfer operation: {}",
                operation
            )))
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
            )))
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
pub fn list_storage_schemas() -> Result<Vec<StorageKindSchema>, CoreError> {
    openhsb_core::schema::list_storage_schemas()
}
