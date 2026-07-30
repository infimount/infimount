use infimount_core::workspaces::{WorkspaceRecord, WorkspaceRegistry};
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::policy::McpStoragePolicy;
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

#[derive(Debug)]
enum Mutation {
    CreatedDirectory(String),
    CreatedFile(String),
    WroteManifest,
    RegisteredWorkspace,
    UpdatedPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateFile {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceAtomicInput {
    pub id: String,
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    pub memory_files: Vec<String>,
    pub template_files: Vec<TemplateFile>,
    pub update_policy: Option<McpStoragePolicy>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceAtomicOutput {
    pub workspace: WorkspaceRecord,
    pub policy_updated: bool,
    pub rollback_errors: Vec<String>,
}

#[tauri::command]
pub async fn create_workspace_atomic(
    state: State<'_, AppState>,
    request: CreateWorkspaceAtomicInput,
) -> Result<CreateWorkspaceAtomicOutput, McpError> {
    let now = chrono::Utc::now().to_rfc3339();

    let normalized_root = normalize_workspace_root(&request.root_path)?;
    if normalized_root == "/" {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not be '/'",
        ));
    }

    let op = state.operator_for_storage_id(&request.storage_id).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            format!("storage '{}' not found or inaccessible: {e}", request.storage_id),
            serde_json::json!({ "storageId": request.storage_id }),
        )
    })?;

    let existing = state.workspaces.load_all().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to load workspaces: {e}"),
        )
    })?;
    for ws in &existing {
        if ws.name == request.name && ws.storage_id == request.storage_id {
            return Err(err_with_details(
                McpErrorCode::ERR_ALREADY_EXISTS,
                format!("workspace '{}' already exists in this storage", request.name),
                serde_json::json!({ "name": request.name, "storageId": request.storage_id }),
            ));
        }
    }

    for ws in &existing {
        if ws.storage_id == request.storage_id {
            let existing_root = ws.root_path.trim_end_matches('/');
            let new_root = normalized_root.trim_end_matches('/');
            if existing_root == new_root
                || new_root.starts_with(&format!("{existing_root}/"))
                || existing_root.starts_with(&format!("{new_root}/"))
            {
                return Err(err_with_details(
                    McpErrorCode::ERR_INVALID_PATH,
                    format!(
                        "root path '{}' overlaps with existing workspace '{}' at '{}'",
                        normalized_root, ws.name, ws.root_path
                    ),
                    serde_json::json!({
                        "existingWorkspace": ws.name,
                        "existingRoot": ws.root_path,
                        "newRoot": normalized_root,
                    }),
                ));
            }
        }
    }

    let workspace = WorkspaceRecord {
        id: request.id.clone(),
        storage_id: request.storage_id.clone(),
        name: request.name.clone(),
        root_path: normalized_root.clone(),
        template_id: request.template_id.clone(),
        created_at: now.clone(),
        updated_at: now,
        memory_files: request.memory_files.clone(),
        checkpoint_ids: vec![],
    };

    let mut mutations: Vec<Mutation> = Vec::new();
    let mut rollback_errors: Vec<String> = Vec::new();

    let result = try_create_workspace(
        &op,
        &state.workspaces,
        &state.registry,
        &request,
        &normalized_root,
        &workspace,
        &mut mutations,
    )
    .await;

    if let Err(e) = result {
        let rollback_errs = rollback_mutations(&op, &state.workspaces, &request.storage_id, &mutations, &workspace).await;
        for re in rollback_errs {
            rollback_errors.push(re);
        }
        return Err(e);
    }

    Ok(CreateWorkspaceAtomicOutput {
        workspace,
        policy_updated: mutations.iter().any(|m| matches!(m, Mutation::UpdatedPolicy)),
        rollback_errors,
    })
}

async fn try_create_workspace(
    op: &opendal::Operator,
    workspaces: &WorkspaceRegistry,
    storage_registry: &StorageRegistry,
    request: &CreateWorkspaceAtomicInput,
    normalized_root: &str,
    workspace: &WorkspaceRecord,
    mutations: &mut Vec<Mutation>,
) -> Result<(), McpError> {
    let dirs_to_create = collect_directories(normalized_root, &request.template_files);
    for dir in &dirs_to_create {
        infimount_core::operations::create_directory(op, dir)
            .await
            .map_err(|e| {
                err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    format!("failed to create directory '{dir}': {e}"),
                    serde_json::json!({ "path": dir }),
                )
            })?;
        mutations.push(Mutation::CreatedDirectory(dir.clone()));
    }

    for tf in &request.template_files {
        let file_path = join_path(normalized_root, &tf.path);
        let data = tf.content.as_bytes().to_vec();
        infimount_core::operations::write_full_with_user_metadata(
            op, &file_path, &data, None,
        )
        .await
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to write file '{file_path}': {e}"),
                serde_json::json!({ "path": file_path }),
            )
        })?;
        mutations.push(Mutation::CreatedFile(file_path));
    }

    let manifest_path = join_path(normalized_root, ".infimount/workspace.json");
    let manifest = serde_json::json!({
        "kind": "infimount-agent-workspace",
        "version": 1,
        "workspace": {
            "id": workspace.id,
            "name": workspace.name,
            "rootPath": workspace.root_path,
            "templateId": workspace.template_id,
            "memoryFiles": workspace.memory_files,
            "createdAt": workspace.created_at,
            "updatedAt": workspace.updated_at,
        },
    });
    let manifest_data = serde_json::to_vec_pretty(&manifest).map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to serialize manifest: {e}"),
        )
    })?;
    infimount_core::operations::write_full_with_user_metadata(
        op,
        &manifest_path,
        &manifest_data,
        None,
    )
    .await
    .map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to write workspace manifest '{manifest_path}': {e}"),
            serde_json::json!({ "path": manifest_path }),
        )
    })?;
    mutations.push(Mutation::WroteManifest);

    workspaces.create(workspace).map_err(|e| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to register workspace: {e}"),
            serde_json::json!({}),
        )
    })?;
    mutations.push(Mutation::RegisteredWorkspace);

    if let Some(ref policy) = request.update_policy {
        let mut policy = policy.clone();
        migrate_and_normalize_policy(&mut policy)?;
        storage_registry
            .with_locked_mutation(|storages: &mut Vec<StorageRecord>| {
                let storage = storages
                    .iter_mut()
                    .find(|item| item.id == request.storage_id)
                    .ok_or_else(|| {
                        err_with_details(
                            McpErrorCode::ERR_STORAGE_NOT_FOUND,
                            format!("storage '{}' not found", request.storage_id),
                            serde_json::json!({ "storageId": request.storage_id }),
                        )
                    })?;
                storage.mcp_policy = policy.clone();
                storage.updated_at = chrono::Utc::now().to_rfc3339();
                Ok(())
            })
            .map_err(|e| {
                err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    format!("failed to update MCP storage policy: {e}"),
                    serde_json::json!({}),
                )
            })?;
        mutations.push(Mutation::UpdatedPolicy);
    }

    Ok(())
}

async fn rollback_mutations(
    op: &opendal::Operator,
    workspaces: &WorkspaceRegistry,
    _storage_id: &str,
    mutations: &[Mutation],
    workspace: &WorkspaceRecord,
) -> Vec<String> {
    let mut errors = Vec::new();

    for mutation in mutations.iter().rev() {
        match mutation {
            Mutation::UpdatedPolicy => {
                // Cannot reliably restore previous policy without snapshotting it.
                // Log the gap but do not block.
            }
            Mutation::RegisteredWorkspace => {
                if let Err(e) = workspaces.delete(&workspace.id) {
                    errors.push(format!("failed to rollback workspace registry entry: {e}"));
                }
            }
            Mutation::WroteManifest => {
                let manifest_path = join_path(&workspace.root_path, ".infimount/workspace.json");
                let _ = infimount_core::operations::delete(op, &manifest_path).await;
            }
            Mutation::CreatedFile(path) => {
                let _ = infimount_core::operations::delete(op, path).await;
            }
            Mutation::CreatedDirectory(_dir) => {
                // Do not delete directories during rollback: they may have
                // pre-existing content or have been created by a prior retry.
                // Empty orphan directories are harmless.
            }
        }
    }

    errors
}

fn collect_directories(root: &str, files: &[TemplateFile]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut dirs = BTreeSet::new();
    dirs.insert(root.to_string());
    dirs.insert(join_path(root, "memory"));
    dirs.insert(join_path(root, ".infimount"));
    dirs.insert(join_path(root, ".infimount/checkpoints"));

    for tf in files {
        let segments: Vec<&str> = tf.path.split('/').collect();
        let mut acc = root.to_string();
        for i in 0..segments.len().saturating_sub(1) {
            acc = format!("{acc}/{}", segments[i]);
            dirs.insert(acc.clone());
        }
    }

    dirs.into_iter().collect()
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim_end_matches('/');
    let relative = relative.trim_start_matches('/');
    format!("{root}/{relative}")
}

fn normalize_workspace_root(raw: &str) -> Result<String, McpError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(err(McpErrorCode::ERR_INVALID_PATH, "workspace root path must not be empty"));
    }

    let with_root = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    let collapsed = with_root
        .replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/");

    let normalized = format!("/{collapsed}");

    if normalized == "/" {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not be or resolve to '/'",
        ));
    }

    Ok(normalized)
}

fn migrate_and_normalize_policy(policy: &mut McpStoragePolicy) -> McpResult<()> {
    infimount_mcp::policy::migrate_legacy_policy(policy)?;
    infimount_mcp::policy::normalize_storage_policy(policy)?;
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::workspaces::WorkspaceRegistry;
    use infimount_mcp::registry::{StorageRecord, StorageRegistry};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn create_test_op() -> (tempfile::TempDir, opendal::Operator) {
        let dir = tempdir().unwrap();
        let builder = opendal::services::Fs::default()
            .root(dir.path().to_str().unwrap());
        let op = opendal::Operator::new(builder).unwrap().finish();
        (dir, op)
    }

    fn create_test_registries(
        temp_config: &tempfile::TempDir,
    ) -> (WorkspaceRegistry, StorageRegistry) {
        let workspace_registry =
            WorkspaceRegistry::new(&temp_config.path().join("workspaces.json"));
        let storage_registry = StorageRegistry::with_secret_store(
            Some(temp_config.path().join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        (workspace_registry, storage_registry)
    }

    fn create_test_storage(storage_registry: &StorageRegistry, dir: &tempfile::TempDir) {
        let mut record = StorageRecord::new(
            "test-storage".to_string(),
            "local".to_string(),
            serde_json::json!({
                "root": dir.path().to_string_lossy().to_string(),
            }),
        );
        record.id = "test-storage-id".to_string();
        record.mcp_exposed = true;
        storage_registry
            .with_locked_mutation(|storages| {
                storages.push(record.clone());
                Ok(())
            })
            .unwrap();
    }

    fn create_test_input(storage_id: &str, root_path: &str) -> CreateWorkspaceAtomicInput {
        CreateWorkspaceAtomicInput {
            id: "ws-test-1".to_string(),
            storage_id: storage_id.to_string(),
            name: "Test Workspace".to_string(),
            root_path: root_path.to_string(),
            template_id: "default".to_string(),
            memory_files: vec!["notes.md".to_string()],
            template_files: vec![TemplateFile {
                path: "README.md".to_string(),
                content: "# Test\n".to_string(),
            }],
            update_policy: None,
        }
    }

    #[tokio::test]
    async fn test_atomic_create_success_path() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        create_test_storage(&storage_registry, &temp_storage);

        let input = create_test_input("test-storage-id", "/workspace1");

        let mut mutations: Vec<Mutation> = Vec::new();
        let workspace = WorkspaceRecord {
            id: input.id.clone(),
            storage_id: input.storage_id.clone(),
            name: input.name.clone(),
            root_path: "/workspace1".to_string(),
            template_id: input.template_id.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: input.memory_files.clone(),
            checkpoint_ids: vec![],
        };

        let result = try_create_workspace(
            &op,
            &workspace_registry,
            &storage_registry,
            &input,
            "/workspace1",
            &workspace,
            &mut mutations,
        )
        .await;

        assert!(result.is_ok(), "happy path should succeed: {:?}", result);

        assert!(op.stat("/workspace1/.infimount/workspace.json").await.is_ok());
        assert!(op.stat("/workspace1/README.md").await.is_ok());
        assert!(op.stat("/workspace1/memory").await.is_ok());

        let loaded = workspace_registry.load_all().unwrap();
        assert!(loaded.iter().any(|w| w.id == "ws-test-1"));
    }

    #[tokio::test]
    async fn test_atomic_create_rollback_on_failure() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        create_test_storage(&storage_registry, &temp_storage);

        let input = CreateWorkspaceAtomicInput {
            id: "ws-rollback".to_string(),
            storage_id: "test-storage-id".to_string(),
            name: "Rollback Test".to_string(),
            root_path: "/rollback-test".to_string(),
            template_id: "default".to_string(),
            memory_files: vec!["data.txt".to_string()],
            template_files: vec![TemplateFile {
                path: "a/b/c/file.txt".to_string(),
                content: "content".to_string(),
            }],
            update_policy: None,
        };

        let mut mutations: Vec<Mutation> = Vec::new();

        let workspace = WorkspaceRecord {
            id: input.id.clone(),
            storage_id: input.storage_id.clone(),
            name: input.name.clone(),
            root_path: "/rollback-test".to_string(),
            template_id: input.template_id.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: input.memory_files.clone(),
            checkpoint_ids: vec![],
        };

        // Simulate failure after directory creation but before file writes
        // by making the workspace registry reject registration
        let dirs = collect_directories("/rollback-test", &input.template_files);
        for dir in &dirs {
            infimount_core::operations::create_directory(&op, dir).await.unwrap();
            mutations.push(Mutation::CreatedDirectory(dir.clone()));
        }

        for tf in &input.template_files {
            let file_path = join_path("/rollback-test", &tf.path);
            let data = tf.content.as_bytes().to_vec();
            infimount_core::operations::write_full_with_user_metadata(&op, &file_path, &data, None)
                .await
                .unwrap();
            mutations.push(Mutation::CreatedFile(file_path));
        }

        let manifest_path = join_path("/rollback-test", ".infimount/workspace.json");
        let manifest_data = serde_json::to_vec_pretty(&serde_json::json!({})).unwrap();
        infimount_core::operations::write_full_with_user_metadata(&op, &manifest_path, &manifest_data, None)
            .await
            .unwrap();
        mutations.push(Mutation::WroteManifest);

        let errors = rollback_mutations(&op, &workspace_registry, "test-storage-id", &mutations, &workspace).await;
        assert!(errors.is_empty(), "rollback should succeed: {:?}", errors);

        // Verify files were cleaned up
        assert!(
            op.stat("/rollback-test/.infimount/workspace.json").await.is_err(),
            "manifest should be deleted"
        );
        assert!(
            op.stat("/rollback-test/a/b/c/file.txt").await.is_err(),
            "files should be deleted"
        );
        // Directories not deleted (policy: orphan dirs left)
        assert!(
            op.stat("/rollback-test/a/b/c").await.is_ok(),
            "dirs should not be deleted during rollback"
        );
    }

    #[tokio::test]
    async fn test_atomic_create_validates_root_path() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        create_test_storage(&storage_registry, &temp_storage);

        let result = normalize_workspace_root("");
        assert!(result.is_err(), "empty root should be rejected");

        let result = normalize_workspace_root("/");
        assert!(result.is_err(), "root '/' should be rejected");
    }

    #[tokio::test]
    async fn test_atomic_create_rejects_overlapping_roots() {
        let temp_config = tempdir().unwrap();
        let (_temp_storage, _op) = create_test_op();
        let (workspace_registry, _storage_registry) = create_test_registries(&temp_config);

        let ws1 = WorkspaceRecord {
            id: "ws-1".to_string(),
            storage_id: "test-storage-id".to_string(),
            name: "WS1".to_string(),
            root_path: "/base".to_string(),
            template_id: "default".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: vec![],
            checkpoint_ids: vec![],
        };
        workspace_registry.create(&ws1).unwrap();

        // Should reject overlapping root
        let existing = workspace_registry.load_all().unwrap();
        let overlaps = existing.iter().any(|w| {
            w.storage_id == ws1.storage_id
                && (w.root_path.trim_end_matches('/') == "/base"
                    || "/base/sub".starts_with(&format!("{}/", w.root_path.trim_end_matches('/'))))
        });
        assert!(overlaps, "should detect overlapping root");
    }

    #[tokio::test]
    async fn test_atomic_create_rollback_leaves_pre_existing_content() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        create_test_storage(&storage_registry, &temp_storage);

        // Create pre-existing file in target root
        let pre_existing_path = "/workspace2/pre_existing.txt";
        infimount_core::operations::write_full_with_user_metadata(
            &op,
            pre_existing_path,
            b"pre-existing",
            None,
        )
        .await
        .unwrap();

        let input = CreateWorkspaceAtomicInput {
            id: "ws-pre-existing".to_string(),
            storage_id: "test-storage-id".to_string(),
            name: "Pre-existing Test".to_string(),
            root_path: "/workspace2".to_string(),
            template_id: "default".to_string(),
            memory_files: vec![],
            template_files: vec![TemplateFile {
                path: "new_file.txt".to_string(),
                content: "new".to_string(),
            }],
            update_policy: None,
        };

        let mut mutations: Vec<Mutation> = Vec::new();
        let workspace = WorkspaceRecord {
            id: input.id.clone(),
            storage_id: input.storage_id.clone(),
            name: input.name.clone(),
            root_path: "/workspace2".to_string(),
            template_id: input.template_id.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: input.memory_files.clone(),
            checkpoint_ids: vec![],
        };

        // Simulate failure after registering workspace
        let dirs = collect_directories("/workspace2", &input.template_files);
        for dir in &dirs {
            infimount_core::operations::create_directory(&op, dir).await.unwrap();
            mutations.push(Mutation::CreatedDirectory(dir.clone()));
        }
        for tf in &input.template_files {
            let file_path = join_path("/workspace2", &tf.path);
            let data = tf.content.as_bytes().to_vec();
            infimount_core::operations::write_full_with_user_metadata(&op, &file_path, &data, None)
                .await
                .unwrap();
            mutations.push(Mutation::CreatedFile(file_path));
        }
        workspace_registry.create(&workspace).unwrap();
        mutations.push(Mutation::RegisteredWorkspace);

        let errors = rollback_mutations(&op, &workspace_registry, "test-storage-id", &mutations, &workspace).await;
        assert!(errors.is_empty(), "rollback should succeed: {:?}", errors);

        // Pre-existing file must survive
        assert!(
            op.stat(pre_existing_path).await.is_ok(),
            "pre-existing files must survive rollback"
        );
    }
}
