use tauri::State;

use crate::state::AppState;
use openhsb_core::{operations, Entry, Source};

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, String> {
    let op = state
        .registry
        .get_operator(&sourceId)
        .await
        .map_err(|e| e.to_string())?;

    operations::list_entries(&op, &path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stat_entry(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Entry, String> {
    let op = state
        .registry
        .get_operator(&sourceId)
        .await
        .map_err(|e| e.to_string())?;

    operations::stat_entry(&op, &path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<u8>, String> {
    let op = state
        .registry
        .get_operator(&sourceId)
        .await
        .map_err(|e| e.to_string())?;

    operations::read_full(&op, &path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let op = state
        .registry
        .get_operator(&sourceId)
        .await
        .map_err(|e| e.to_string())?;

    operations::write_full(&op, &path, &data)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), String> {
    let op = state
        .registry
        .get_operator(&sourceId)
        .await
        .map_err(|e| e.to_string())?;

    operations::delete(&op, &path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> Result<Vec<Source>, String> {
    let sources = state.registry.list_sources().await;
    Ok(sources)
}

#[tauri::command]
pub async fn replace_sources(
    state: State<'_, AppState>,
    sources: Vec<Source>,
) -> Result<(), String> {
    state
        .registry
        .replace_sources(sources)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_source(state: State<'_, AppState>, source: Source) -> Result<(), String> {
    state
        .registry
        .add_source(source)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_source(
    state: State<'_, AppState>,
    sourceId: String,
) -> Result<(), String> {
    state
        .registry
        .remove_source(&sourceId)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_source(state: State<'_, AppState>, source: Source) -> Result<(), String> {
    state
        .registry
        .update_source(source)
        .await
        .map_err(|e| e.to_string())
}
