use infimount_core::workspaces::{
    generate_workspace_id, known_template_plans, memory_files_for, template_files_for,
    validate_workspace_metadata, WorkspaceRecord, WorkspaceRegistry, MAX_CHECKPOINT_IDS,
    WORKSPACE_RECORD_SCHEMA_VERSION,
};
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::policy::{McpAccessMode, McpPathRule, McpRuleSource};
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

#[derive(Debug)]
enum Mutation {
    CreatedDirectory(String),
    WroteTemplateFile { path: String },
    WroteManifest,
    RegisteredWorkspace,
    UpdatedPolicy(PolicySnapshot),
}

#[derive(Debug, Clone)]
struct PolicySnapshot {
    rule: Option<McpPathRule>,
    applied_rule: Option<McpPathRule>,
    revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceAtomicInput {
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    pub adopt_existing: Option<bool>,
    pub access_profile: Option<String>,
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
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let now = chrono::Utc::now().to_rfc3339();

    let workspace_name = request.name.trim();
    if workspace_name.is_empty()
        || workspace_name.len() > 200
        || workspace_name.chars().any(char::is_control)
    {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace name must be 1-200 characters without control characters",
        ));
    }

    if !known_template_plans().contains(&request.template_id.as_str()) {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!(
                "unknown template plan '{}'; expected one of {:?}",
                request.template_id,
                known_template_plans()
            ),
        ));
    }

    let access_profile = request.access_profile.as_deref().unwrap_or("read_only");
    let access_mode = parse_access_profile(access_profile)?;

    let normalized_root = validate_workspace_root(&request.root_path)?;

    let storage = state.find_storage_by_id(&request.storage_id)?;
    if !storage.enabled {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_DISABLED,
            format!("storage '{}' is disabled", request.storage_id),
            serde_json::json!({ "storageId": request.storage_id }),
        ));
    }

    let op = state
        .operator_for_storage_id(&request.storage_id)
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!(
                    "storage '{}' not found or inaccessible: {e}",
                    request.storage_id
                ),
                serde_json::json!({ "storageId": request.storage_id }),
            )
        })?;

    let caps = op.info().full_capability();
    if !caps.stat || !caps.read || !caps.write || !caps.create_dir || !caps.list {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!(
                "storage '{}' does not support required operations (stat={}, read={}, write={}, create_dir={}, list={})",
                request.storage_id, caps.stat, caps.read, caps.write, caps.create_dir, caps.list
            ),
            serde_json::json!({ "storageId": request.storage_id }),
        ));
    }

    let existing = state.workspaces.load_all().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to load workspaces: {e}"),
        )
    })?;
    for ws in &existing {
        if ws.name == workspace_name && ws.storage_id == request.storage_id {
            return Err(err_with_details(
                McpErrorCode::ERR_ALREADY_EXISTS,
                format!(
                    "workspace '{}' already exists in this storage",
                    workspace_name
                ),
                serde_json::json!({ "name": workspace_name, "storageId": request.storage_id }),
            ));
        }
    }

    for ws in &existing {
        if ws.storage_id == request.storage_id {
            let existing_root = ws.root_path.trim_end_matches('/');
            if normalized_root == existing_root
                || normalized_root.starts_with(&format!("{existing_root}/"))
                || existing_root.starts_with(&format!("{normalized_root}/"))
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

    let root_exists = stat_exists(&op, &normalized_root).await?;
    if root_exists {
        let is_empty = is_directory_empty(&op, &normalized_root).await?;
        if !is_empty {
            let adopt = request.adopt_existing.unwrap_or(false);
            if !adopt {
                return Err(err_with_details(
                    McpErrorCode::ERR_INVALID_PATH,
                    format!(
                        "workspace root '{}' already exists and is not empty; set adoptExisting=true to adopt",
                        normalized_root
                    ),
                    serde_json::json!({ "rootPath": normalized_root }),
                ));
            }
            let existing_manifest = stat_exists(
                &op,
                &join_path(&normalized_root, ".infimount/workspace.json"),
            )
            .await?;
            if existing_manifest {
                return Err(err_with_details(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    format!(
                        "workspace root '{}' already contains a workspace manifest; cannot adopt",
                        normalized_root
                    ),
                    serde_json::json!({ "rootPath": normalized_root }),
                ));
            }
        }
    }

    let workspace_id = generate_workspace_id();
    let policy_rule_id = Some(format!("workspace:{workspace_id}"));

    let workspace = WorkspaceRecord {
        id: workspace_id.clone(),
        schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
        storage_id: request.storage_id.clone(),
        name: workspace_name.to_string(),
        root_path: normalized_root.clone(),
        template_id: request.template_id.clone(),
        access_profile: access_profile.to_string(),
        policy_rule_id: policy_rule_id.clone(),
        created_at: now.clone(),
        updated_at: now,
        memory_files: memory_files_for(&request.template_id),
        checkpoint_ids: vec![],
    };

    let mut mutations: Vec<Mutation> = Vec::new();

    let result = try_create_workspace(
        &op,
        &state.workspaces,
        &state.registry,
        &request.storage_id,
        &normalized_root,
        &workspace,
        access_mode,
        &mut mutations,
    )
    .await;

    if let Err(mut e) = result {
        let rollback_errs = rollback_mutations(
            &op,
            &state.workspaces,
            &state.registry,
            &request.storage_id,
            &workspace,
            &mutations,
        )
        .await;
        if !rollback_errs.is_empty() {
            e.details = serde_json::json!({ "rollbackErrors": rollback_errs });
        }
        return Err(e);
    }

    let mut event = infimount_mcp::telemetry::ProductEvent::new(
        infimount_mcp::telemetry::ProductEventName::WorkspaceCreated,
    );
    event.workspace_template = Some(workspace.template_id.clone());
    event.access_profile = Some(workspace.access_profile.clone());
    event.success = Some(true);
    let _ = state.product_events.record(event);

    Ok(CreateWorkspaceAtomicOutput {
        workspace,
        policy_updated: mutations
            .iter()
            .any(|m| matches!(m, Mutation::UpdatedPolicy(..))),
        rollback_errors: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn try_create_workspace(
    op: &opendal::Operator,
    workspaces: &WorkspaceRegistry,
    storage_registry: &StorageRegistry,
    storage_id: &str,
    normalized_root: &str,
    workspace: &WorkspaceRecord,
    access_mode: McpAccessMode,
    mutations: &mut Vec<Mutation>,
) -> Result<(), McpError> {
    let dirs = required_directories(normalized_root, &workspace.template_id);
    for dir in &dirs {
        let exists = stat_exists(op, dir).await?;
        if !exists {
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
    }

    let template_files = template_files_for(&workspace.template_id);
    for tf in &template_files {
        let file_path = join_path(normalized_root, &tf.path);
        let exists = stat_exists(op, &file_path).await?;
        if !exists {
            let data = tf.content.as_bytes().to_vec();
            infimount_core::operations::write_full(op, &file_path, &data)
                .await
                .map_err(|e| {
                    err_with_details(
                        McpErrorCode::ERR_INTERNAL,
                        format!("failed to write template file '{file_path}': {e}"),
                        serde_json::json!({ "path": file_path }),
                    )
                })?;
            mutations.push(Mutation::WroteTemplateFile {
                path: file_path.clone(),
            });
        }
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
            "accessProfile": workspace.access_profile,
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

    apply_workspace_policy_rule(
        storage_registry,
        storage_id,
        workspace,
        access_mode,
        mutations,
    )?;

    Ok(())
}

fn parse_access_profile(profile: &str) -> McpResult<McpAccessMode> {
    match profile {
        "none" => Ok(McpAccessMode::None),
        "read_only" => Ok(McpAccessMode::ReadOnly),
        "read_write" => Ok(McpAccessMode::ReadWrite),
        other => Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("unknown access profile '{other}'; expected none, read_only, or read_write"),
        )),
    }
}

fn validate_workspace_root(raw: &str) -> McpResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not be empty",
        ));
    }

    // reject encoded traversal sequences before normalization
    let lower = trimmed.to_lowercase();
    if lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not contain URL-encoded path separators or dot sequences (e.g. %2e, %2f, %5c)",
        ));
    }

    // reject backslashes
    if trimmed.contains('\\') {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not contain backslashes",
        ));
    }

    // reject all control characters (including newline, carriage return, tab)
    for ch in trimmed.chars() {
        if ch.is_control() {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace root path must not contain control characters",
                serde_json::json!({ "char": ch as u32 }),
            ));
        }
    }

    // convert backslashes to forward slashes (for the rest of processing)
    let with_slashes = trimmed.replace('\\', "/");

    // split into segments and validate
    let segments: Vec<&str> = with_slashes.split('/').collect();
    let mut clean_segments: Vec<String> = Vec::new();

    for segment in &segments {
        if segment.is_empty() {
            // leading/trailing empty segments from the split are fine (they produce leading/trailing /)
            continue;
        }
        if *segment == "." {
            continue;
        }
        if *segment == ".." {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace root path must not contain '..' segments",
            ));
        }
        clean_segments.push((*segment).to_string());
    }

    if clean_segments.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not be or resolve to '/'",
        ));
    }

    let normalized = format!("/{}", clean_segments.join("/"));

    if normalized == "/" {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root path must not be or resolve to '/'",
        ));
    }

    Ok(normalized)
}

fn apply_workspace_policy_rule(
    storage_registry: &StorageRegistry,
    storage_id: &str,
    workspace: &WorkspaceRecord,
    access_mode: McpAccessMode,
    mutations: &mut Vec<Mutation>,
) -> McpResult<()> {
    let rule_id = workspace
        .policy_rule_id
        .clone()
        .unwrap_or_else(|| format!("workspace:{}", workspace.id));

    // Capture the exact policy state under the same registry lock as the mutation.
    let mut snapshot = None;
    storage_registry
        .with_locked_mutation(|storages: &mut Vec<StorageRecord>| {
            let storage = storages
                .iter_mut()
                .find(|item| item.id == storage_id)
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_STORAGE_NOT_FOUND,
                        format!("storage '{storage_id}' not found"),
                        serde_json::json!({ "storageId": storage_id }),
                    )
                })?;

            let previous_rule = storage
                .mcp_policy
                .rules
                .iter()
                .find(|rule| rule.id == rule_id)
                .cloned();

            let rule = McpPathRule {
                id: rule_id.clone(),
                prefix: workspace.root_path.clone(),
                access: access_mode,
                source: McpRuleSource::Workspace {
                    workspace_id: workspace.id.clone(),
                },
                confirmation_rules: None,
            };
            snapshot = Some(PolicySnapshot {
                rule: previous_rule,
                applied_rule: Some(rule.clone()),
                revision: storage.revision,
            });

            let existing_idx = storage
                .mcp_policy
                .rules
                .iter()
                .position(|r| r.id == rule_id);
            if let Some(idx) = existing_idx {
                storage.mcp_policy.rules[idx] = rule;
            } else {
                storage.mcp_policy.rules.push(rule);
            }

            storage.revision = storage.revision.saturating_add(1);
            storage.updated_at = chrono::Utc::now().to_rfc3339();
            Ok(())
        })
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to apply workspace policy rule: {e}"),
                serde_json::json!({}),
            )
        })?;

    mutations.push(Mutation::UpdatedPolicy(snapshot.ok_or_else(|| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to snapshot workspace policy",
        )
    })?));
    Ok(())
}

fn remove_workspace_policy_rule(
    storage_registry: &StorageRegistry,
    storage_id: &str,
    workspace_id: &str,
) -> McpResult<PolicySnapshot> {
    let rule_id = format!("workspace:{workspace_id}");
    let mut snapshot = None;
    storage_registry
        .with_locked_mutation(|storages: &mut Vec<StorageRecord>| {
            let storage = storages
                .iter_mut()
                .find(|item| item.id == storage_id)
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_STORAGE_NOT_FOUND,
                        format!("storage '{storage_id}' not found"),
                        serde_json::json!({ "storageId": storage_id }),
                    )
                })?;

            snapshot = Some(PolicySnapshot {
                rule: storage
                    .mcp_policy
                    .rules
                    .iter()
                    .find(|rule| rule.id == rule_id)
                    .cloned(),
                applied_rule: None,
                revision: storage.revision,
            });
            storage.mcp_policy.rules.retain(|r| r.id != rule_id);

            storage.revision = storage.revision.saturating_add(1);
            storage.updated_at = chrono::Utc::now().to_rfc3339();
            Ok(())
        })
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to remove workspace policy rule: {e}"),
                serde_json::json!({}),
            )
        })?;
    snapshot.ok_or_else(|| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to snapshot workspace policy",
        )
    })
}

fn policy_rules_equivalent(left: Option<&McpPathRule>, right: Option<&McpPathRule>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.id == right.id
                && left.prefix.trim_matches('/') == right.prefix.trim_matches('/')
                && left.access == right.access
                && left.source == right.source
                && left.confirmation_rules == right.confirmation_rules
        }
        _ => false,
    }
}

fn restore_policy_rule(
    storage_registry: &StorageRegistry,
    storage_id: &str,
    rule_id: &str,
    snapshot: &PolicySnapshot,
) -> McpResult<()> {
    storage_registry
        .with_locked_mutation(|storages: &mut Vec<StorageRecord>| {
            let storage = storages
                .iter_mut()
                .find(|item| item.id == storage_id)
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_STORAGE_NOT_FOUND,
                        format!("storage '{storage_id}' not found during policy restore"),
                        serde_json::json!({ "storageId": storage_id }),
                    )
                })?;
            let current_rule = storage
                .mcp_policy
                .rules
                .iter()
                .find(|rule| rule.id == rule_id)
                .cloned();
            if !policy_rules_equivalent(current_rule.as_ref(), snapshot.applied_rule.as_ref()) {
                return Err(err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    "workspace policy rule changed after mutation; refusing unsafe rollback",
                    serde_json::json!({
                        "snapshotRevision": snapshot.revision,
                        "actualRevision": storage.revision,
                    }),
                ));
            }
            if let Some(rule) = &snapshot.rule {
                let idx = storage
                    .mcp_policy
                    .rules
                    .iter()
                    .position(|r| r.id == rule_id);
                if let Some(i) = idx {
                    storage.mcp_policy.rules[i] = rule.clone();
                } else {
                    storage.mcp_policy.rules.push(rule.clone());
                }
            } else {
                storage.mcp_policy.rules.retain(|r| r.id != rule_id);
            }
            // Rollback is itself a new mutation; never move a persisted revision backwards.
            storage.revision = storage.revision.saturating_add(1);
            storage.updated_at = chrono::Utc::now().to_rfc3339();
            Ok(())
        })
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to restore policy rule: {e}"),
                serde_json::json!({}),
            )
        })?;
    Ok(())
}

async fn rollback_mutations(
    op: &opendal::Operator,
    workspaces: &WorkspaceRegistry,
    storage_registry: &StorageRegistry,
    storage_id: &str,
    workspace: &WorkspaceRecord,
    mutations: &[Mutation],
) -> Vec<String> {
    let mut errors = Vec::new();

    for mutation in mutations.iter().rev() {
        match mutation {
            Mutation::UpdatedPolicy(snapshot) => {
                let rule_id = workspace
                    .policy_rule_id
                    .clone()
                    .unwrap_or_else(|| format!("workspace:{}", workspace.id));
                if restore_policy_rule(storage_registry, storage_id, &rule_id, snapshot).is_err() {
                    errors.push("ERR_WORKSPACE_ROLLBACK_POLICY".to_string());
                }
            }
            Mutation::WroteTemplateFile { path } => {
                if let Err(error) = infimount_core::operations::delete(op, path).await {
                    if !matches!(&error, infimount_core::CoreError::Storage(inner) if inner.kind() == opendal::ErrorKind::NotFound)
                    {
                        errors.push("ERR_WORKSPACE_ROLLBACK_TEMPLATE".to_string());
                    }
                }
            }
            Mutation::RegisteredWorkspace => {
                if let Err(e) = workspaces.delete(&workspace.id) {
                    errors.push(format!("failed to rollback workspace registry entry: {e}"));
                }
            }
            Mutation::WroteManifest => {
                let manifest_path = join_path(&workspace.root_path, ".infimount/workspace.json");
                if let Err(error) = infimount_core::operations::delete(op, &manifest_path).await {
                    if !matches!(&error, infimount_core::CoreError::Storage(inner) if inner.kind() == opendal::ErrorKind::NotFound)
                    {
                        errors.push("ERR_WORKSPACE_ROLLBACK_MANIFEST".to_string());
                    }
                }
            }
            Mutation::CreatedDirectory(dir) => {
                if let Err(error) = op.delete(dir).await {
                    if error.kind() != opendal::ErrorKind::NotFound {
                        errors.push("ERR_WORKSPACE_ROLLBACK_DIRECTORY".to_string());
                    }
                }
            }
        }
    }

    errors
}

async fn stat_exists(op: &opendal::Operator, path: &str) -> Result<bool, McpError> {
    match op.stat(path).await {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to stat path '{path}': {e}"),
            serde_json::json!({ "kind": format!("{:?}", e.kind()), "path": path }),
        )),
    }
}

fn required_directories(root: &str, template_id: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut dirs = BTreeSet::new();
    dirs.insert(root.to_string());
    dirs.insert(join_path(root, ".infimount"));
    dirs.insert(join_path(root, ".infimount/checkpoints"));
    if template_id == "admin" {
        // admin template has no memory dir
    } else {
        dirs.insert(join_path(root, "memory"));
    }
    dirs.into_iter().collect()
}

async fn is_directory_empty(op: &opendal::Operator, path: &str) -> Result<bool, McpError> {
    let path = path.trim_end_matches('/');
    infimount_core::operations::list_entries(op, path)
        .await
        .map(|entries| entries.is_empty())
        .map_err(|error| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to inspect existing workspace root",
                serde_json::json!({ "kind": format!("{error:?}") }),
            )
        })
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim_end_matches('/');
    let relative = relative.trim_start_matches('/');
    format!("{root}/{relative}")
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
pub struct UpdateWorkspaceRequest {
    pub id: String,
    pub name: Option<String>,
    pub access_profile: Option<String>,
    pub memory_files: Option<Vec<String>>,
    pub checkpoint_ids: Option<Vec<String>>,
}

#[tauri::command]
pub async fn update_workspace(
    state: State<'_, AppState>,
    request: UpdateWorkspaceRequest,
) -> Result<WorkspaceRecord, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let registry = &state.workspaces;
    let _transaction = registry.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let mut workspace = registry
        .find_by_id(&request.id)
        .map_err(|e| {
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
        let name = name.trim();
        if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace name must be 1-200 characters without control characters",
            ));
        }
        workspace.name = name.to_string();
    }

    // root_path is immutable after creation
    if request.access_profile.is_some() && workspace.root_path.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace root_path must be set before access_profile can be updated",
        ));
    }

    let new_access_mode = request
        .access_profile
        .as_deref()
        .map(parse_access_profile)
        .transpose()?;
    if let Some(ref profile) = request.access_profile {
        workspace.access_profile = profile.clone();
    }
    if let Some(memory_files) = request.memory_files {
        workspace.memory_files = memory_files;
    }
    if let Some(checkpoint_ids) = request.checkpoint_ids {
        if checkpoint_ids.len() > MAX_CHECKPOINT_IDS {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                format!("workspace may contain at most {MAX_CHECKPOINT_IDS} checkpoints"),
            ));
        }
        workspace.checkpoint_ids = checkpoint_ids;
    }
    validate_workspace_metadata(&workspace).map_err(|e| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("invalid workspace metadata: {e}"),
        )
    })?;
    workspace.updated_at = chrono::Utc::now().to_rfc3339();

    let mut policy_snapshot: Option<PolicySnapshot> = None;
    if let Some(access_mode) = new_access_mode {
        let mut mutations = Vec::new();
        apply_workspace_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &workspace,
            access_mode,
            &mut mutations,
        )?;
        policy_snapshot = mutations.into_iter().find_map(|mutation| match mutation {
            Mutation::UpdatedPolicy(snapshot) => Some(snapshot),
            _ => None,
        });
    }

    if let Err(e) = registry.update(&workspace) {
        // Roll back policy if it was changed
        let rollback_error = policy_snapshot.and_then(|snapshot| {
            let rule_id = workspace
                .policy_rule_id
                .clone()
                .unwrap_or_else(|| format!("workspace:{}", workspace.id));
            restore_policy_rule(&state.registry, &workspace.storage_id, &rule_id, &snapshot)
                .err()
                .map(|_| "ERR_WORKSPACE_ROLLBACK_POLICY")
        });
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to update workspace: {e}"),
            serde_json::json!({ "rollbackError": rollback_error }),
        ));
    }

    Ok(workspace)
}

#[tauri::command]
pub async fn delete_workspace(state: State<'_, AppState>, id: String) -> Result<(), McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let workspace = state
        .workspaces
        .find_by_id(&id)
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to find workspace: {e}"),
                serde_json::json!({}),
            )
        })?
        .ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("workspace '{id}' not found"),
                serde_json::json!({ "workspaceId": id }),
            )
        })?;

    let rule_id = workspace
        .policy_rule_id
        .clone()
        .unwrap_or_else(|| format!("workspace:{id}"));
    let policy_snapshot =
        remove_workspace_policy_rule(&state.registry, &workspace.storage_id, &id)?;

    if let Err(error) = state.workspaces.delete(&id) {
        let rollback_error = restore_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &rule_id,
            &policy_snapshot,
        )
        .err()
        .map(|_| "ERR_WORKSPACE_ROLLBACK_POLICY");
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to delete workspace: {error}"),
            serde_json::json!({
                "workspaceId": id,
                "rollbackError": rollback_error,
            }),
        ));
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkspaceWithFilesRequest {
    pub id: String,
    pub confirm_delete_files: bool,
}

fn require_delete_files_confirmation(confirmed: bool) -> McpResult<()> {
    if confirmed {
        Ok(())
    } else {
        Err(err(
            McpErrorCode::ERR_CONFIRMATION_REQUIRED,
            "explicit confirmation is required to delete workspace files",
        ))
    }
}

#[tauri::command]
pub async fn delete_workspace_with_files(
    state: State<'_, AppState>,
    request: DeleteWorkspaceWithFilesRequest,
) -> Result<(), McpError> {
    require_delete_files_confirmation(request.confirm_delete_files)?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let workspace = state
        .workspaces
        .find_by_id(&request.id)
        .map_err(|e| {
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to find workspace: {e}"),
            )
        })?
        .ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                "workspace was not found",
                serde_json::json!({ "workspaceId": request.id }),
            )
        })?;
    let op = state
        .operator_for_storage_id(&workspace.storage_id)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                "workspace storage is unavailable",
            )
        })?;
    let rule_id = workspace
        .policy_rule_id
        .clone()
        .unwrap_or_else(|| format!("workspace:{}", workspace.id));
    let policy_snapshot =
        remove_workspace_policy_rule(&state.registry, &workspace.storage_id, &workspace.id)?;
    if let Err(error) = state.workspaces.delete(&workspace.id) {
        let rollback_error = restore_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &rule_id,
            &policy_snapshot,
        )
        .err()
        .map(|_| "ERR_WORKSPACE_ROLLBACK_POLICY");
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to remove workspace registration: {error}"),
            serde_json::json!({ "rollbackError": rollback_error }),
        ));
    }

    if let Err(delete_error) = infimount_core::operations::delete(&op, &workspace.root_path).await {
        let registry_rollback = state
            .workspaces
            .create(&workspace)
            .err()
            .map(|_| "ERR_WORKSPACE_ROLLBACK_REGISTRY");
        let policy_rollback = restore_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &rule_id,
            &policy_snapshot,
        )
        .err()
        .map(|_| "ERR_WORKSPACE_ROLLBACK_POLICY");
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("workspace file deletion failed: {delete_error}"),
            serde_json::json!({
                "workspaceId": workspace.id,
                "registryRollbackError": registry_rollback,
                "policyRollbackError": policy_rollback,
                "dataMayBePartiallyDeleted": true,
            }),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLegacyWorkspacesRequest {
    pub workspaces: Vec<WorkspaceRecord>,
}

fn require_per_workspace_legacy_import(count: usize) -> McpResult<()> {
    if count <= 1 {
        Ok(())
    } else {
        Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspaces must be imported one at a time so failures remain independently retryable",
        ))
    }
}

#[tauri::command]
pub async fn import_legacy_workspaces(
    state: State<'_, AppState>,
    request: ImportLegacyWorkspacesRequest,
) -> Result<usize, McpError> {
    require_per_workspace_legacy_import(request.workspaces.len())?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let existing = state.workspaces.load_all().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to load workspace registry",
        )
    })?;
    let existing_ids = existing
        .iter()
        .map(|workspace| workspace.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut validated: Vec<WorkspaceRecord> = Vec::new();

    for mut workspace in request.workspaces {
        if existing_ids.contains(workspace.id.as_str()) {
            continue;
        }
        if workspace.id.trim().is_empty()
            || workspace.id.len() > 200
            || workspace.name.trim().is_empty()
            || workspace.name.len() > 200
            || workspace.id.chars().any(char::is_control)
            || workspace.name.chars().any(char::is_control)
        {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "legacy workspace contains an invalid identifier or name",
            ));
        }
        workspace.root_path = validate_workspace_root(&workspace.root_path)?;
        if !known_template_plans().contains(&workspace.template_id.as_str()) {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "legacy workspace contains an unknown template",
            ));
        }
        parse_access_profile(&workspace.access_profile)?;
        let storage = state.find_storage_by_id(&workspace.storage_id)?;
        if !storage.enabled {
            return Err(err(
                McpErrorCode::ERR_STORAGE_DISABLED,
                "legacy workspace references a disabled storage",
            ));
        }
        workspace.schema_version = WORKSPACE_RECORD_SCHEMA_VERSION;
        workspace.policy_rule_id = Some(format!("workspace:{}", workspace.id));
        workspace.memory_files = memory_files_for(&workspace.template_id);
        validate_workspace_metadata(&workspace).map_err(|e| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                format!("invalid legacy workspace metadata: {e}"),
            )
        })?;

        if validated.iter().any(|other| other.id == workspace.id) {
            return Err(err(
                McpErrorCode::ERR_ALREADY_EXISTS,
                "legacy import contains duplicate workspace identifiers",
            ));
        }

        for other in existing.iter().chain(validated.iter()) {
            if other.storage_id != workspace.storage_id {
                continue;
            }
            let other_root = other.root_path.trim_end_matches('/');
            if workspace.root_path == other_root
                || workspace.root_path.starts_with(&format!("{other_root}/"))
                || other_root.starts_with(&format!("{}/", workspace.root_path))
            {
                return Err(err(
                    McpErrorCode::ERR_INVALID_PATH,
                    "legacy workspace root overlaps an existing workspace",
                ));
            }
        }

        verify_legacy_workspace_manifest(&state, &workspace).await?;
        validated.push(workspace);
    }

    let imported = state
        .workspaces
        .import_legacy(validated.clone())
        .map_err(|e| {
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to import legacy workspaces: {e}"),
            )
        })?;

    let mut applied: Vec<(String, String, PolicySnapshot)> = Vec::new();
    for workspace in &validated {
        let access = parse_access_profile(&workspace.access_profile)?;
        let mut mutations = Vec::new();
        if let Err(error) = apply_workspace_policy_rule(
            &state.registry,
            &workspace.storage_id,
            workspace,
            access,
            &mut mutations,
        ) {
            let mut rollback_errors = Vec::new();
            for imported_workspace in validated.iter().rev() {
                if state.workspaces.delete(&imported_workspace.id).is_err() {
                    rollback_errors.push("ERR_WORKSPACE_ROLLBACK_REGISTRY");
                }
            }
            for (storage_id, rule_id, snapshot) in applied.iter().rev() {
                if restore_policy_rule(&state.registry, storage_id, rule_id, snapshot).is_err() {
                    rollback_errors.push("ERR_WORKSPACE_ROLLBACK_POLICY");
                }
            }
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to create policy for migrated workspace: {error}"),
                serde_json::json!({ "rollbackErrors": rollback_errors }),
            ));
        }
        if let Some(Mutation::UpdatedPolicy(snapshot)) = mutations.pop() {
            applied.push((
                workspace.storage_id.clone(),
                workspace.policy_rule_id.clone().unwrap_or_default(),
                snapshot,
            ));
        }
    }

    Ok(imported)
}

async fn verify_legacy_workspace_manifest(
    state: &AppState,
    workspace: &WorkspaceRecord,
) -> McpResult<()> {
    let op = state
        .operator_for_storage_id(&workspace.storage_id)
        .map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("legacy workspace storage is inaccessible: {e}"),
                serde_json::json!({ "storageId": workspace.storage_id }),
            )
        })?;
    let root_metadata = op.stat(&workspace.root_path).await.map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace root does not exist or is inaccessible",
            serde_json::json!({ "rootPath": workspace.root_path }),
        )
    })?;
    if !root_metadata.is_dir() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace root is not a directory",
        ));
    }

    let manifest_path = join_path(&workspace.root_path, ".infimount/workspace.json");
    let metadata = op.stat(&manifest_path).await.map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace manifest is missing or inaccessible",
            serde_json::json!({ "manifestPath": manifest_path }),
        )
    })?;
    if metadata.is_dir() || metadata.content_length() > 64 * 1024 {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace manifest is invalid",
        ));
    }
    let bytes = infimount_core::operations::read_full(&op, &manifest_path)
        .await
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "failed to read legacy workspace manifest",
            )
        })?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace manifest is malformed",
        )
    })?;
    if !manifest_matches_workspace(&manifest, workspace) {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "legacy workspace manifest identity does not match the imported record",
        ));
    }
    Ok(())
}

fn manifest_matches_workspace(manifest: &serde_json::Value, workspace: &WorkspaceRecord) -> bool {
    let identity = manifest
        .get("workspace")
        .and_then(serde_json::Value::as_object);
    manifest.get("kind").and_then(serde_json::Value::as_str) == Some("infimount-agent-workspace")
        && identity
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some(workspace.id.as_str())
        && identity
            .and_then(|value| value.get("rootPath"))
            .and_then(serde_json::Value::as_str)
            == Some(workspace.root_path.as_str())
        && identity
            .and_then(|value| value.get("templateId"))
            .and_then(serde_json::Value::as_str)
            == Some(workspace.template_id.as_str())
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
        let builder = opendal::services::Fs::default().root(dir.path().to_str().unwrap());
        let op = opendal::Operator::new(builder).unwrap().finish();
        (dir, op)
    }

    fn create_test_registries(
        temp_config: &tempfile::TempDir,
    ) -> (WorkspaceRegistry, StorageRegistry) {
        let workspace_registry = WorkspaceRegistry::new(temp_config.path());
        let storage_registry = StorageRegistry::with_secret_store(
            Some(temp_config.path().join("registry.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        (workspace_registry, storage_registry)
    }

    fn create_test_storage(storage_registry: &StorageRegistry, dir: &tempfile::TempDir) -> String {
        let mut record = StorageRecord::new(
            "test-storage".to_string(),
            "local".to_string(),
            serde_json::json!({
                "root": dir.path().to_string_lossy().to_string(),
            }),
        );
        let storage_id = "test-storage-id".to_string();
        record.id = storage_id.clone();
        record.mcp_exposed = true;
        record.enabled = true;
        storage_registry
            .with_locked_mutation(|storages| {
                storages.push(record.clone());
                Ok(())
            })
            .unwrap();
        storage_id
    }

    fn make_workspace_record(id: &str, storage_id: &str, root_path: &str) -> WorkspaceRecord {
        WorkspaceRecord {
            id: id.to_string(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage_id.to_string(),
            name: format!("Test {id}"),
            root_path: root_path.to_string(),
            template_id: "coding".to_string(),
            access_profile: "read_write".to_string(),
            policy_rule_id: Some(format!("workspace:{id}")),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: memory_files_for("coding"),
            checkpoint_ids: vec![],
        }
    }

    #[tokio::test]
    async fn test_atomic_create_success_path() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);

        let root_path = "/workspace1";
        let ws = make_workspace_record("ws-test-1", &storage_id, root_path);

        let mut mutations: Vec<Mutation> = Vec::new();
        let result = try_create_workspace(
            &op,
            &workspace_registry,
            &storage_registry,
            &storage_id,
            root_path,
            &ws,
            McpAccessMode::ReadWrite,
            &mut mutations,
        )
        .await;

        assert!(result.is_ok(), "happy path should succeed: {:?}", result);
        assert!(op
            .stat("/workspace1/.infimount/workspace.json")
            .await
            .is_ok());
        assert!(op.stat("/workspace1/memory").await.is_ok());

        let loaded = workspace_registry.load_all().unwrap();
        assert!(loaded.iter().any(|w| w.id == "ws-test-1"));
    }

    #[tokio::test]
    async fn test_atomic_create_rollback_on_failure() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);

        let root_path = "/rollback-test";
        let ws = make_workspace_record("ws-rollback", &storage_id, root_path);

        let mut mutations: Vec<Mutation> = Vec::new();

        let dirs = required_directories(root_path, "coding");
        for dir in &dirs {
            infimount_core::operations::create_directory(&op, dir)
                .await
                .unwrap();
            mutations.push(Mutation::CreatedDirectory(dir.clone()));
        }

        let manifest_path = join_path(root_path, ".infimount/workspace.json");
        let manifest_data = serde_json::to_vec_pretty(&serde_json::json!({})).unwrap();
        infimount_core::operations::write_full_with_user_metadata(
            &op,
            &manifest_path,
            &manifest_data,
            None,
        )
        .await
        .unwrap();
        mutations.push(Mutation::WroteManifest);

        let errors = rollback_mutations(
            &op,
            &workspace_registry,
            &storage_registry,
            &storage_id,
            &ws,
            &mutations,
        )
        .await;
        assert!(errors.is_empty(), "rollback should succeed: {:?}", errors);

        assert!(
            op.stat("/rollback-test/.infimount/workspace.json")
                .await
                .is_err(),
            "manifest should be deleted"
        );
        assert!(
            op.stat("/rollback-test/memory").await.is_err(),
            "directories created by the transaction should be deleted during rollback"
        );
    }

    #[tokio::test]
    async fn test_parse_access_profile() {
        assert!(matches!(
            parse_access_profile("none"),
            Ok(McpAccessMode::None)
        ));
        assert!(matches!(
            parse_access_profile("read_only"),
            Ok(McpAccessMode::ReadOnly)
        ));
        assert!(matches!(
            parse_access_profile("read_write"),
            Ok(McpAccessMode::ReadWrite)
        ));
        assert!(parse_access_profile("invalid").is_err());
    }

    #[test]
    fn test_validate_workspace_root() {
        assert_eq!(validate_workspace_root("/foo/bar").unwrap(), "/foo/bar");
        assert_eq!(validate_workspace_root("foo/bar").unwrap(), "/foo/bar");
        assert_eq!(validate_workspace_root("/foo//bar/.").unwrap(), "/foo/bar");
        assert!(validate_workspace_root("").is_err());
        assert!(validate_workspace_root("/").is_err());
        assert!(validate_workspace_root("/../foo").is_err());
        assert!(validate_workspace_root("foo/..").is_err());
        assert!(validate_workspace_root("/foo%2fbar").is_err());
        assert!(validate_workspace_root("/foo%5cbar").is_err());
        assert!(validate_workspace_root("/foo%2ebar").is_err());
        assert!(validate_workspace_root("/foo\\bar").is_err());
    }

    #[test]
    fn legacy_import_is_forced_to_report_outcomes_per_workspace() {
        require_per_workspace_legacy_import(0).unwrap();
        require_per_workspace_legacy_import(1).unwrap();
        let error = require_per_workspace_legacy_import(2).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_PATH);
    }

    #[test]
    fn deleting_workspace_files_requires_explicit_confirmation() {
        let error = require_delete_files_confirmation(false).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_CONFIRMATION_REQUIRED);
        require_delete_files_confirmation(true).unwrap();
    }

    #[test]
    fn legacy_manifest_requires_matching_identity() {
        let workspace = make_workspace_record("workspace-1", "storage-1", "/root");
        let valid = serde_json::json!({
            "kind": "infimount-agent-workspace",
            "workspace": {
                "id": "workspace-1",
                "rootPath": "/root",
                "templateId": "coding",
            }
        });
        assert!(manifest_matches_workspace(&valid, &workspace));
        let mismatched = serde_json::json!({
            "kind": "infimount-agent-workspace",
            "workspace": {
                "id": "different",
                "rootPath": "/root",
                "templateId": "coding",
            }
        });
        assert!(!manifest_matches_workspace(&mismatched, &workspace));
    }

    #[tokio::test]
    async fn test_policy_rule_applied_and_removed() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);

        let ws_id = "ws-policy-test";
        let ws = make_workspace_record(ws_id, &storage_id, "/policy-test");

        let mut mutations = Vec::new();
        apply_workspace_policy_rule(
            &storage_registry,
            &storage_id,
            &ws,
            McpAccessMode::ReadWrite,
            &mut mutations,
        )
        .expect("apply policy rule");

        let storages = storage_registry.load_all().unwrap();
        let storage = storages.iter().find(|s| s.id == storage_id).unwrap();
        assert_eq!(storage.mcp_policy.rules.len(), 1);
        assert_eq!(storage.mcp_policy.rules[0].id, "workspace:ws-policy-test");
        assert_eq!(storage.revision, 2);

        remove_workspace_policy_rule(&storage_registry, &storage_id, ws_id)
            .expect("remove policy rule");

        let storages = storage_registry.load_all().unwrap();
        let storage = storages.iter().find(|s| s.id == storage_id).unwrap();
        assert!(storage.mcp_policy.rules.is_empty());
        assert_eq!(storage.revision, 3);
    }

    #[test]
    fn policy_rollback_preserves_unrelated_concurrent_mutation() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record("rollback-cas", &storage_id, "/cas");
        let mut mutations = Vec::new();
        apply_workspace_policy_rule(
            &storage_registry,
            &storage_id,
            &workspace,
            McpAccessMode::ReadOnly,
            &mut mutations,
        )
        .expect("apply workspace policy");
        let Mutation::UpdatedPolicy(snapshot) = mutations.pop().unwrap() else {
            panic!("missing policy snapshot");
        };
        storage_registry
            .with_locked_mutation(|storages| {
                let storage = storages
                    .iter_mut()
                    .find(|item| item.id == storage_id)
                    .unwrap();
                storage.read_only = true;
                storage.revision += 1;
                Ok(())
            })
            .expect("concurrent storage mutation");

        restore_policy_rule(
            &storage_registry,
            &storage_id,
            "workspace:rollback-cas",
            &snapshot,
        )
        .expect("targeted rollback should preserve unrelated changes");
        let storage = storage_registry
            .load_all()
            .unwrap()
            .into_iter()
            .find(|item| item.id == storage_id)
            .unwrap();
        assert!(storage.read_only);
        assert!(storage.mcp_policy.rules.is_empty());
        assert_eq!(storage.revision, 4);
    }

    #[test]
    fn policy_rollback_refuses_to_overwrite_changed_target_rule() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record("rollback-target", &storage_id, "/target");
        let mut mutations = Vec::new();
        apply_workspace_policy_rule(
            &storage_registry,
            &storage_id,
            &workspace,
            McpAccessMode::ReadOnly,
            &mut mutations,
        )
        .expect("apply workspace policy");
        let Mutation::UpdatedPolicy(snapshot) = mutations.pop().unwrap() else {
            panic!("missing policy snapshot");
        };
        storage_registry
            .with_locked_mutation(|storages| {
                let storage = storages
                    .iter_mut()
                    .find(|item| item.id == storage_id)
                    .unwrap();
                let rule = storage
                    .mcp_policy
                    .rules
                    .iter_mut()
                    .find(|rule| rule.id == "workspace:rollback-target")
                    .unwrap();
                rule.access = McpAccessMode::ReadWrite;
                storage.revision += 1;
                Ok(())
            })
            .expect("concurrent target mutation");

        assert!(restore_policy_rule(
            &storage_registry,
            &storage_id,
            "workspace:rollback-target",
            &snapshot,
        )
        .is_err());
        let storage = storage_registry
            .load_all()
            .unwrap()
            .into_iter()
            .find(|item| item.id == storage_id)
            .unwrap();
        assert_eq!(storage.mcp_policy.rules[0].access, McpAccessMode::ReadWrite);
        assert_eq!(storage.revision, 3);
    }
}
