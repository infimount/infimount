use infimount_core::workspaces::{WorkspaceRecord, WorkspaceRegistry};
use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode};
use serde::Deserialize;
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn list_workspaces(state: State<'_, AppState>) -> Result<Vec<WorkspaceRecord>, McpError> {
    state.workspaces.load_all().map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to list workspaces: {e}"),
            serde_json::json!({}),
        )
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceRequest {
    pub id: String,
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    pub memory_files: Vec<String>,
}

#[tauri::command]
pub fn create_workspace(
    state: State<'_, AppState>,
    request: CreateWorkspaceRequest,
) -> Result<WorkspaceRecord, McpError> {
    let now = chrono::Utc::now().to_rfc3339();
    let workspace = WorkspaceRecord {
        id: request.id,
        storage_id: request.storage_id,
        name: request.name,
        root_path: request.root_path,
        template_id: request.template_id,
        created_at: now.clone(),
        updated_at: now,
        memory_files: request.memory_files,
        checkpoint_ids: vec![],
    };

    let registry = &state.workspaces;
    // Check for duplicate name within same storage
    let existing = registry.load_all().map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to load workspaces: {e}"),
            serde_json::json!({}),
        )
    })?;
    for ws in &existing {
        if ws.name == workspace.name && ws.storage_id == workspace.storage_id {
            return Err(err_with_details(
                McpErrorCode::ERR_ALREADY_EXISTS,
                format!("workspace '{}' already exists in this storage", workspace.name),
                serde_json::json!({ "name": workspace.name, "storageId": workspace.storage_id }),
            ));
        }
    }

    registry.create(&workspace).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to create workspace: {e}"),
            serde_json::json!({}),
        )
    })?;

    Ok(workspace)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceRequest {
    pub id: String,
    pub name: Option<String>,
    pub root_path: Option<String>,
    pub memory_files: Option<Vec<String>>,
    pub checkpoint_ids: Option<Vec<String>>,
}

#[tauri::command]
pub fn update_workspace(
    state: State<'_, AppState>,
    request: UpdateWorkspaceRequest,
) -> Result<WorkspaceRecord, McpError> {
    let registry = &state.workspaces;
    let mut workspace = registry.find_by_id(&request.id).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to find workspace: {e}"),
            serde_json::json!({}),
        )
    })?
    .ok_or_else(|| {
        err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            format!("workspace '{}' not found", request.id),
            serde_json::json!({ "workspaceId": request.id }),
        )
    })?;

    if let Some(name) = request.name {
        workspace.name = name;
    }
    if let Some(root_path) = request.root_path {
        workspace.root_path = root_path;
    }
    if let Some(memory_files) = request.memory_files {
        workspace.memory_files = memory_files;
    }
    if let Some(checkpoint_ids) = request.checkpoint_ids {
        workspace.checkpoint_ids = checkpoint_ids;
    }
    workspace.updated_at = chrono::Utc::now().to_rfc3339();

    registry.update(&workspace).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to update workspace: {e}"),
            serde_json::json!({}),
        )
    })?;

    Ok(workspace)
}

#[tauri::command]
pub fn delete_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), McpError> {
    state.workspaces.delete(&id).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            format!("failed to delete workspace '{id}': {e}"),
            serde_json::json!({ "workspaceId": id }),
        )
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLegacyWorkspacesRequest {
    pub workspaces: Vec<WorkspaceRecord>,
}

#[tauri::command]
pub fn import_legacy_workspaces(
    state: State<'_, AppState>,
    request: ImportLegacyWorkspacesRequest,
) -> Result<usize, McpError> {
    state.workspaces.import_legacy(request.workspaces).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to import legacy workspaces: {e}"),
            serde_json::json!({}),
        )
    })
}
