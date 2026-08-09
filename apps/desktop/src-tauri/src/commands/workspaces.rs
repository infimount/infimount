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

const MAX_CHECKPOINT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_CHECKPOINT_TOTAL_BYTES: u64 = 5 * 1024 * 1024;
const MAX_CHECKPOINT_MANIFEST_BYTES: u64 = 6 * 1024 * 1024;
const MAX_CHECKPOINT_LABEL_BYTES: usize = 200;
const MAX_LEGACY_MANIFEST_BYTES: u64 = 64 * 1024;

async fn read_workspace_file_bounded(
    op: &opendal::Operator,
    path: &str,
    max_bytes: u64,
    missing_code: McpErrorCode,
    read_message: &'static str,
) -> McpResult<Vec<u8>> {
    let before = op
        .stat(path)
        .await
        .map_err(|_| err(missing_code, read_message))?;
    if before.is_dir() || before.content_length() > max_bytes {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace file exceeds its bounded size",
        ));
    }
    let result = infimount_core::operations::read_file_range(op, path, 0, max_bytes)
        .await
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, read_message))?;
    let after = op
        .stat(path)
        .await
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, read_message))?;
    if after.is_dir()
        || after.content_length() != before.content_length()
        || result.truncated
        || result.total_size != after.content_length()
        || u64::try_from(result.bytes.len()).unwrap_or(u64::MAX) != after.content_length()
    {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace file changed while being read or exceeds its bounded size",
        ));
    }
    Ok(result.bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckpointFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckpoint {
    #[serde(default = "default_checkpoint_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub workspace_id: String,
    pub created_at: String,
    pub label: String,
    pub manifest_path: String,
    pub memory_files: Vec<WorkspaceCheckpointFile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckpointSummary {
    pub schema_version: u32,
    pub id: String,
    pub workspace_id: String,
    pub created_at: String,
    pub label: String,
    pub manifest_path: String,
    pub file_count: usize,
}

impl From<&WorkspaceCheckpoint> for WorkspaceCheckpointSummary {
    fn from(checkpoint: &WorkspaceCheckpoint) -> Self {
        Self {
            schema_version: checkpoint.schema_version,
            id: checkpoint.id.clone(),
            workspace_id: checkpoint.workspace_id.clone(),
            created_at: checkpoint.created_at.clone(),
            label: checkpoint.label.clone(),
            manifest_path: checkpoint.manifest_path.clone(),
            file_count: checkpoint.memory_files.len(),
        }
    }
}

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
    #[serde(default = "default_true")]
    pub apply_policy: bool,
}

fn default_true() -> bool {
    true
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
    state.require_operational()?;
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

    let caps = op.info().capability();
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
    let policy_rule_id = request
        .apply_policy
        .then(|| format!("workspace:{workspace_id}"));

    let workspace = WorkspaceRecord {
        id: workspace_id.clone(),
        schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
        storage_id: request.storage_id.clone(),
        name: workspace_name.to_string(),
        root_path: normalized_root.clone(),
        template_id: request.template_id.clone(),
        access_profile: if request.apply_policy {
            access_profile.to_string()
        } else {
            "none".to_string()
        },
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
        request.apply_policy,
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
    apply_policy: bool,
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

    if apply_policy {
        apply_workspace_policy_rule(
            storage_registry,
            storage_id,
            workspace,
            access_mode,
            mutations,
        )?;
    }

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

fn resolve_legacy_policy_rule_id(storage: &StorageRecord, workspace: &WorkspaceRecord) -> String {
    if let Some(rule_id) = workspace.policy_rule_id.as_deref() {
        return rule_id.to_string();
    }
    let normalized_root = workspace.root_path.trim_matches('/');
    // v0.8 workspaces used ws:<id>; old records may lack the newly persisted
    // policyRuleId, so bind by the source identity before considering adoption.
    storage
        .mcp_policy
        .rules
        .iter()
        .find(|rule| {
            rule.prefix.trim_matches('/') == normalized_root
                && matches!(
                    &rule.source,
                    McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
                )
        })
        .or_else(|| {
            storage.mcp_policy.rules.iter().find(|rule| {
                rule.prefix.trim_matches('/') == normalized_root
                    && matches!(rule.source, McpRuleSource::Manual)
            })
        })
        .map(|rule| rule.id.clone())
        .unwrap_or_else(|| format!("workspace:{}", workspace.id))
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
            if let Some(previous) = &previous_rule {
                let source_matches = matches!(&previous.source, McpRuleSource::Manual)
                    || matches!(
                        &previous.source,
                        McpRuleSource::Workspace { workspace_id }
                            if workspace_id == &workspace.id
                    );
                if previous.prefix.trim_matches('/') != workspace.root_path.trim_matches('/')
                    || !source_matches
                {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_POLICY,
                        "stored workspace policy rule identity does not match workspace metadata",
                    ));
                }
            }

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

            if storage.mcp_policy.rules.iter().any(|existing| {
                existing.id != rule_id
                    && existing.prefix.trim_matches('/') == workspace.root_path.trim_matches('/')
            }) {
                return Err(err(
                    McpErrorCode::ERR_ALREADY_EXISTS,
                    "another policy rule already grants this workspace root",
                ));
            }
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
    workspace: &WorkspaceRecord,
) -> McpResult<PolicySnapshot> {
    let rule_id = workspace
        .policy_rule_id
        .as_deref()
        .unwrap_or("")
        .to_string();
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
            let rule = storage
                .mcp_policy
                .rules
                .iter()
                .find(|rule| rule.id == rule_id)
                .cloned()
                .ok_or_else(|| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "workspace policy rule is missing; refusing to revoke an unknown grant",
                    )
                })?;
            if rule.prefix.trim_matches('/') != workspace.root_path.trim_matches('/')
                || !matches!(
                    rule.source,
                    McpRuleSource::Workspace { ref workspace_id } if workspace_id == &workspace.id
                )
            {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "workspace policy rule identity does not match; refusing deletion",
                ));
            }
            snapshot = Some(PolicySnapshot {
                rule: Some(rule),
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
    state.require_operational()?;
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
}

#[tauri::command]
pub async fn update_workspace(
    state: State<'_, AppState>,
    request: UpdateWorkspaceRequest,
) -> Result<WorkspaceRecord, McpError> {
    state.require_operational()?;
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
    validate_workspace_metadata(&workspace).map_err(|e| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("invalid workspace metadata: {e}"),
        )
    })?;
    workspace.updated_at = chrono::Utc::now().to_rfc3339();

    let mut policy_snapshot: Option<PolicySnapshot> = None;
    if let Some(access_mode) = new_access_mode {
        if workspace.policy_rule_id.is_none() {
            let storage = state.find_storage_by_id(&workspace.storage_id)?;
            workspace.policy_rule_id = Some(resolve_legacy_policy_rule_id(&storage, &workspace));
        }
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
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let mut workspace = state
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

    if workspace.access_profile != "none" && workspace.policy_rule_id.is_none() {
        let storage = state.find_storage_by_id(&workspace.storage_id)?;
        workspace.policy_rule_id = Some(resolve_legacy_policy_rule_id(&storage, &workspace));
    }
    let rule_id = workspace.policy_rule_id.clone().unwrap_or_default();
    let policy_snapshot = if workspace.access_profile == "none" {
        None
    } else {
        Some(remove_workspace_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &workspace,
        )?)
    };

    if let Err(error) = state.workspaces.delete(&id) {
        let rollback_error = policy_snapshot
            .as_ref()
            .and_then(|snapshot| {
                restore_policy_rule(&state.registry, &workspace.storage_id, &rule_id, snapshot)
                    .err()
            })
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
    state.require_operational()?;
    require_delete_files_confirmation(request.confirm_delete_files)?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let mut workspace = state
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
    if workspace.access_profile != "none" && workspace.policy_rule_id.is_none() {
        let storage = state.find_storage_by_id(&workspace.storage_id)?;
        workspace.policy_rule_id = Some(resolve_legacy_policy_rule_id(&storage, &workspace));
    }
    let rule_id = workspace.policy_rule_id.clone().unwrap_or_default();
    let policy_snapshot = if workspace.access_profile == "none" {
        None
    } else {
        Some(remove_workspace_policy_rule(
            &state.registry,
            &workspace.storage_id,
            &workspace,
        )?)
    };
    if let Err(error) = state.workspaces.delete(&workspace.id) {
        let rollback_error = policy_snapshot
            .as_ref()
            .and_then(|snapshot| {
                restore_policy_rule(&state.registry, &workspace.storage_id, &rule_id, snapshot)
                    .err()
            })
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
        let policy_rollback = policy_snapshot
            .as_ref()
            .and_then(|snapshot| {
                restore_policy_rule(&state.registry, &workspace.storage_id, &rule_id, snapshot)
                    .err()
            })
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
    state.require_operational()?;
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
        workspace.memory_files = memory_files_for(&workspace.template_id);
        workspace.policy_rule_id = Some(resolve_legacy_policy_rule_id(&storage, &workspace));
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

    persist_legacy_workspaces_and_policies(&state, &validated)
}

fn persist_legacy_workspaces_and_policies(
    state: &AppState,
    validated: &[WorkspaceRecord],
) -> McpResult<usize> {
    let imported = state
        .workspaces
        .import_legacy(validated.to_vec())
        .map_err(|e| {
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to import legacy workspaces: {e}"),
            )
        })?;

    let mut applied: Vec<(String, String, PolicySnapshot)> = Vec::new();
    for workspace in validated {
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
    let bytes = read_workspace_file_bounded(
        &op,
        &manifest_path,
        MAX_LEGACY_MANIFEST_BYTES,
        McpErrorCode::ERR_INVALID_PATH,
        "legacy workspace manifest is missing, inaccessible, or changed while being read",
    )
    .await?;
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceCheckpointRequest {
    pub workspace_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreWorkspaceCheckpointRequest {
    pub workspace_id: String,
    pub checkpoint_id: String,
    pub confirm_overwrite: bool,
}

fn require_checkpoint_restore_confirmation(
    request: &RestoreWorkspaceCheckpointRequest,
) -> McpResult<()> {
    if request.confirm_overwrite {
        return Ok(());
    }
    Err(err_with_details(
        McpErrorCode::ERR_CONFIRMATION_REQUIRED,
        "restoring a checkpoint overwrites workspace memory files and requires explicit confirmation",
        serde_json::json!({
            "workspaceId": request.workspace_id,
            "checkpointId": request.checkpoint_id,
            "operation": "restore_workspace_checkpoint",
        }),
    ))
}

fn find_workspace_or_error(
    registry: &WorkspaceRegistry,
    workspace_id: &str,
) -> McpResult<WorkspaceRecord> {
    registry
        .find_by_id(workspace_id)
        .map_err(|e| {
            err(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to load workspace registry: {e}"),
            )
        })?
        .ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                "workspace was not found",
                serde_json::json!({ "workspaceId": workspace_id }),
            )
        })
}

fn validate_checkpoint_label(label: Option<&str>) -> McpResult<String> {
    let label = label.unwrap_or("Checkpoint").trim();
    if label.is_empty()
        || label.len() > MAX_CHECKPOINT_LABEL_BYTES
        || label.chars().any(char::is_control)
    {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!(
                "checkpoint label must be 1-{MAX_CHECKPOINT_LABEL_BYTES} bytes without control characters"
            ),
        ));
    }
    Ok(label.to_string())
}

fn default_checkpoint_schema_version() -> u32 {
    1
}

fn valid_checkpoint_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.starts_with("checkpoint-")
        && id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn checkpoint_manifest_relative_path(checkpoint_id: &str) -> McpResult<String> {
    if !valid_checkpoint_id(checkpoint_id) {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid workspace checkpoint ID",
        ));
    }
    Ok(format!(".infimount/checkpoints/{checkpoint_id}.json"))
}

fn validate_checkpoint_context(
    op: &opendal::Operator,
    storage: &StorageRecord,
    workspace: &WorkspaceRecord,
    require_write: bool,
) -> McpResult<()> {
    validate_workspace_metadata(workspace).map_err(|e| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("invalid workspace metadata: {e}"),
        )
    })?;
    let normalized_root = validate_workspace_root(&workspace.root_path)?;
    if normalized_root != workspace.root_path || workspace.storage_id != storage.id {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace registry contains an invalid storage or root binding",
        ));
    }
    if !storage.enabled {
        return Err(err(
            McpErrorCode::ERR_STORAGE_DISABLED,
            "workspace storage is disabled",
        ));
    }
    if require_write && storage.read_only {
        return Err(err(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            "workspace storage is read-only",
        ));
    }
    let capabilities = op.info().capability();
    let supported = capabilities.stat
        && capabilities.read
        && (!require_write || (capabilities.write && capabilities.delete));
    if !supported {
        return Err(err(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            "workspace storage does not support the bounded checkpoint transaction",
        ));
    }
    Ok(())
}

fn validate_checkpoint_manifest(
    checkpoint: &WorkspaceCheckpoint,
    workspace: &WorkspaceRecord,
) -> McpResult<()> {
    if checkpoint.schema_version != 1
        || checkpoint.workspace_id != workspace.id
        || !valid_checkpoint_id(&checkpoint.id)
        || checkpoint.manifest_path != checkpoint_manifest_relative_path(&checkpoint.id)?
    {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace checkpoint manifest identity is invalid",
        ));
    }
    validate_checkpoint_label(Some(&checkpoint.label))?;
    if checkpoint.memory_files.len() != workspace.memory_files.len() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace checkpoint does not contain the trusted memory-file set",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let expected = workspace
        .memory_files
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut total = 0_u64;
    for file in &checkpoint.memory_files {
        if !expected.contains(file.path.as_str()) || !seen.insert(file.path.as_str()) {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint contains an untrusted or duplicate memory path",
            ));
        }
        let len = u64::try_from(file.content.len()).unwrap_or(u64::MAX);
        if len > MAX_CHECKPOINT_FILE_BYTES {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint file exceeds the 1 MiB limit",
            ));
        }
        total = total.checked_add(len).ok_or_else(|| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint size overflow",
            )
        })?;
    }
    if total > MAX_CHECKPOINT_TOTAL_BYTES {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace checkpoint exceeds the 5 MiB total limit",
        ));
    }
    Ok(())
}

async fn read_checkpoint_manifest(
    op: &opendal::Operator,
    workspace: &WorkspaceRecord,
    checkpoint_id: &str,
) -> McpResult<WorkspaceCheckpoint> {
    if !workspace
        .checkpoint_ids
        .iter()
        .any(|id| id == checkpoint_id)
    {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "workspace checkpoint was not found",
            serde_json::json!({ "workspaceId": workspace.id, "checkpointId": checkpoint_id }),
        ));
    }
    let relative = checkpoint_manifest_relative_path(checkpoint_id)?;
    let path = join_path(&workspace.root_path, &relative);
    let bytes = read_workspace_file_bounded(
        op,
        &path,
        MAX_CHECKPOINT_MANIFEST_BYTES,
        McpErrorCode::ERR_STORAGE_NOT_FOUND,
        "workspace checkpoint manifest is missing, inaccessible, or changed while being read",
    )
    .await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace checkpoint manifest is malformed",
        )
    })?;
    // v0.8 development builds wrote an envelope around the checkpoint. Accept it
    // as a one-way compatibility input while keeping all validation server-side.
    let checkpoint_value = value.get("checkpoint").cloned().unwrap_or(value);
    let checkpoint: WorkspaceCheckpoint =
        serde_json::from_value(checkpoint_value).map_err(|_| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint manifest is malformed",
            )
        })?;
    validate_checkpoint_manifest(&checkpoint, workspace)?;
    Ok(checkpoint)
}

async fn create_checkpoint_transaction(
    op: &opendal::Operator,
    registry: &WorkspaceRegistry,
    storage: &StorageRecord,
    mut workspace: WorkspaceRecord,
    label: Option<&str>,
    fail_before_registry_update: bool,
) -> McpResult<WorkspaceCheckpoint> {
    validate_checkpoint_context(op, storage, &workspace, true)?;
    if workspace.checkpoint_ids.len() >= MAX_CHECKPOINT_IDS {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            format!("workspace may contain at most {MAX_CHECKPOINT_IDS} checkpoints"),
        ));
    }

    let checkpoint_id = format!("checkpoint-{}", uuid::Uuid::new_v4());
    let relative_manifest = checkpoint_manifest_relative_path(&checkpoint_id)?;
    let manifest_path = join_path(&workspace.root_path, &relative_manifest);

    let mut memory_files = Vec::with_capacity(workspace.memory_files.len());
    let mut total_bytes = 0_u64;
    for relative in &workspace.memory_files {
        let path = join_path(&workspace.root_path, relative);
        let bytes = read_workspace_file_bounded(
            op,
            &path,
            MAX_CHECKPOINT_FILE_BYTES,
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "workspace memory file is missing, inaccessible, or changed while being read",
        )
        .await?;
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        total_bytes = total_bytes.checked_add(len).ok_or_else(|| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint size overflow",
            )
        })?;
        if total_bytes > MAX_CHECKPOINT_TOTAL_BYTES {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace checkpoint exceeds the 5 MiB total limit",
            ));
        }
        let content = String::from_utf8(bytes.to_vec()).map_err(|_| {
            err(
                McpErrorCode::ERR_INVALID_PATH,
                "workspace memory files must contain valid UTF-8 text",
            )
        })?;
        memory_files.push(WorkspaceCheckpointFile {
            path: relative.clone(),
            content,
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    let checkpoint = WorkspaceCheckpoint {
        schema_version: 1,
        id: checkpoint_id.clone(),
        workspace_id: workspace.id.clone(),
        created_at: now.clone(),
        label: validate_checkpoint_label(label)?,
        manifest_path: relative_manifest,
        memory_files,
    };
    validate_checkpoint_manifest(&checkpoint, &workspace)?;
    let manifest = serde_json::to_vec_pretty(&checkpoint).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize workspace checkpoint",
        )
    })?;
    if u64::try_from(manifest.len()).unwrap_or(u64::MAX) > MAX_CHECKPOINT_MANIFEST_BYTES {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "workspace checkpoint manifest exceeds its bounded size",
        ));
    }

    if let Err(error) = infimount_core::operations::write_full(op, &manifest_path, &manifest).await
    {
        let _ = infimount_core::operations::delete(op, &manifest_path).await;
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to write workspace checkpoint: {error}"),
        ));
    }

    workspace.checkpoint_ids.insert(0, checkpoint_id);
    workspace.updated_at = now;
    let registry_result = if fail_before_registry_update {
        Err(infimount_core::CoreError::Config(
            "injected checkpoint registry failure".to_string(),
        ))
    } else {
        registry.update(&workspace)
    };
    if let Err(error) = registry_result {
        let rollback_error = infimount_core::operations::delete(op, &manifest_path)
            .await
            .err()
            .map(|_| "ERR_WORKSPACE_CHECKPOINT_ROLLBACK_MANIFEST");
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to register workspace checkpoint: {error}"),
            serde_json::json!({ "rollbackError": rollback_error }),
        ));
    }
    Ok(checkpoint)
}

async fn restore_checkpoint_transaction(
    op: &opendal::Operator,
    storage: &StorageRecord,
    workspace: &WorkspaceRecord,
    checkpoint_id: &str,
    fail_before_write_index: Option<usize>,
) -> McpResult<()> {
    validate_checkpoint_context(op, storage, workspace, true)?;
    let checkpoint = read_checkpoint_manifest(op, workspace, checkpoint_id).await?;

    let mut snapshots: Vec<(String, Option<Vec<u8>>)> = Vec::new();
    for file in &checkpoint.memory_files {
        let target = join_path(&workspace.root_path, &file.path);
        let previous = match op.stat(&target).await {
            Ok(metadata) => {
                if metadata.is_dir() || metadata.content_length() > MAX_CHECKPOINT_FILE_BYTES {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_PATH,
                        "workspace target file exceeds the rollback snapshot limit",
                    ));
                }
                let bytes = read_workspace_file_bounded(
                    op,
                    &target,
                    MAX_CHECKPOINT_FILE_BYTES,
                    McpErrorCode::ERR_INTERNAL,
                    "failed to snapshot workspace target before restore",
                )
                .await?;
                Some(bytes)
            }
            Err(error) if error.kind() == opendal::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "failed to inspect workspace target before restore",
                ));
            }
        };
        snapshots.push((target, previous));
    }

    for (index, file) in checkpoint.memory_files.iter().enumerate() {
        let target = &snapshots[index].0;
        let write_result = if fail_before_write_index == Some(index) {
            Err(infimount_core::CoreError::Config(
                "injected checkpoint restore failure".to_string(),
            ))
        } else {
            infimount_core::operations::write_full(op, target, file.content.as_bytes()).await
        };
        if let Err(error) = write_result {
            let mut rollback_errors = Vec::new();
            // A backend may have partially committed the failing write, so restore
            // the current target as well as every previously completed target.
            for (rollback_path, previous) in snapshots[..=index].iter().rev() {
                let rollback = match previous {
                    Some(data) => {
                        infimount_core::operations::write_full(op, rollback_path, data).await
                    }
                    None => infimount_core::operations::delete(op, rollback_path).await,
                };
                if rollback.is_err() {
                    rollback_errors.push("ERR_WORKSPACE_CHECKPOINT_ROLLBACK_FILE");
                }
            }
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                format!("failed to restore workspace checkpoint: {error}"),
                serde_json::json!({ "rollbackErrors": rollback_errors }),
            ));
        }
    }
    Ok(())
}

fn workspace_storage(state: &AppState, workspace: &WorkspaceRecord) -> McpResult<StorageRecord> {
    let storage = state.find_storage_by_id(&workspace.storage_id)?;
    if !storage.enabled {
        return Err(err(
            McpErrorCode::ERR_STORAGE_DISABLED,
            "workspace storage is disabled",
        ));
    }
    Ok(storage)
}

#[tauri::command]
pub async fn list_workspace_checkpoints(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<WorkspaceCheckpointSummary>, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let workspace = find_workspace_or_error(&state.workspaces, &workspace_id)?;
    let storage = workspace_storage(&state, &workspace)?;
    let op = state.operator_for_storage_id(&storage.id).map_err(|_| {
        err(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "workspace storage is inaccessible",
        )
    })?;
    validate_checkpoint_context(&op, &storage, &workspace, false)?;
    let mut checkpoints = Vec::with_capacity(workspace.checkpoint_ids.len());
    for checkpoint_id in &workspace.checkpoint_ids {
        let checkpoint = read_checkpoint_manifest(&op, &workspace, checkpoint_id).await?;
        checkpoints.push(WorkspaceCheckpointSummary::from(&checkpoint));
    }
    checkpoints.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(checkpoints)
}

#[tauri::command]
pub async fn create_workspace_checkpoint(
    state: State<'_, AppState>,
    request: CreateWorkspaceCheckpointRequest,
) -> Result<WorkspaceCheckpointSummary, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let workspace = find_workspace_or_error(&state.workspaces, &request.workspace_id)?;
    let storage = workspace_storage(&state, &workspace)?;
    let op = state.operator_for_storage_id(&storage.id).map_err(|_| {
        err(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "workspace storage is inaccessible",
        )
    })?;
    let checkpoint = create_checkpoint_transaction(
        &op,
        &state.workspaces,
        &storage,
        workspace,
        request.label.as_deref(),
        false,
    )
    .await?;
    Ok(WorkspaceCheckpointSummary::from(&checkpoint))
}

#[tauri::command]
pub async fn restore_workspace_checkpoint(
    state: State<'_, AppState>,
    request: RestoreWorkspaceCheckpointRequest,
) -> Result<(), McpError> {
    state.require_operational()?;
    require_checkpoint_restore_confirmation(&request)?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _transaction = state.workspaces.acquire_mutation_lock().map_err(|e| {
        err(
            McpErrorCode::ERR_INTERNAL,
            format!("failed to lock workspace mutation: {e}"),
        )
    })?;
    let workspace = find_workspace_or_error(&state.workspaces, &request.workspace_id)?;
    let storage = workspace_storage(&state, &workspace)?;
    let op = state.operator_for_storage_id(&storage.id).map_err(|_| {
        err(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "workspace storage is inaccessible",
        )
    })?;
    restore_checkpoint_transaction(&op, &storage, &workspace, &request.checkpoint_id, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::secrets::MemorySecretStore;
    use infimount_core::workspaces::WorkspaceRegistry;
    use infimount_mcp::registry::{StorageRecord, StorageRegistry};
    use infimount_mcp::McpStoragePolicy;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn checkpoint_restore_requires_confirmation_bound_to_exact_request() {
        let denied = RestoreWorkspaceCheckpointRequest {
            workspace_id: "workspace-a".to_string(),
            checkpoint_id: "checkpoint-a".to_string(),
            confirm_overwrite: false,
        };
        let error = require_checkpoint_restore_confirmation(&denied).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_CONFIRMATION_REQUIRED);
        assert_eq!(error.details["workspaceId"], "workspace-a");
        assert_eq!(error.details["checkpointId"], "checkpoint-a");

        let confirmed = RestoreWorkspaceCheckpointRequest {
            confirm_overwrite: true,
            ..denied
        };
        require_checkpoint_restore_confirmation(&confirmed).unwrap();
    }

    fn create_test_op() -> (tempfile::TempDir, opendal::Operator) {
        let dir = tempdir().unwrap();
        let builder = opendal::services::Fs::default().root(dir.path().to_str().unwrap());
        let op = opendal::Operator::new(builder).unwrap();
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
            true,
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

    #[test]
    fn v071_fixture_migration_adopts_exact_manual_rule_identity() {
        let workspaces: Vec<WorkspaceRecord> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/v0.7.1/workspaces-localstorage.json"
        ))
        .unwrap();
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/v0.7.1/workspace-manifest.json"
        ))
        .unwrap();
        let policy: McpStoragePolicy = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/v0.7.1/storage-policy-legacy.json"
        ))
        .unwrap();
        let workspace = &workspaces[0];
        assert!(manifest_matches_workspace(&manifest, workspace));
        let mut storage = StorageRecord::new(
            "Fixture storage".into(),
            "local".into(),
            serde_json::json!({"root": "/tmp"}),
        );
        storage.id = workspace.storage_id.clone();
        storage.mcp_policy = policy;
        assert_eq!(
            resolve_legacy_policy_rule_id(
                &storage,
                &WorkspaceRecord {
                    policy_rule_id: None,
                    ..workspace.clone()
                }
            ),
            "legacy-allowed-workspace"
        );
    }

    #[test]
    fn v071_fixture_runs_persist_policy_adoption_delete_and_restart_path() {
        let dir = tempdir().unwrap();
        let state = AppState::new_for_test(dir.path(), Arc::new(MemorySecretStore::new()));
        let mut workspaces: Vec<WorkspaceRecord> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/v0.7.1/workspaces-localstorage.json"
        ))
        .unwrap();
        let mut storage = StorageRecord::new(
            "Fixture storage".into(),
            "local".into(),
            serde_json::json!({"root": dir.path().to_string_lossy()}),
        );
        storage.id = workspaces[0].storage_id.clone();
        storage.mcp_policy = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/v0.7.1/storage-policy-legacy.json"
        ))
        .unwrap();
        state.registry.save_all_atomic(&[storage]).unwrap();
        workspaces[0].policy_rule_id = None;
        workspaces[0].root_path = "/workspace".into();
        workspaces[0].access_profile = "read_only".into();
        workspaces[0].schema_version = WORKSPACE_RECORD_SCHEMA_VERSION;
        workspaces[0].memory_files = memory_files_for(&workspaces[0].template_id);
        let migrated_storage = state.registry.load_all().unwrap().pop().unwrap();
        workspaces[0].policy_rule_id = Some(resolve_legacy_policy_rule_id(
            &migrated_storage,
            &workspaces[0],
        ));

        persist_legacy_workspaces_and_policies(&state, &workspaces).unwrap();
        let persisted = state.workspaces.load_all().unwrap();
        assert_eq!(persisted[0].policy_rule_id.as_deref(), Some("migrated-0"));
        let storage = state.registry.load_all().unwrap().pop().unwrap();
        assert!(matches!(
            storage.mcp_policy.rules[0].source,
            McpRuleSource::Workspace { ref workspace_id } if workspace_id == &persisted[0].id
        ));

        let workspace = persisted[0].clone();
        remove_workspace_policy_rule(&state.registry, &workspace.storage_id, &workspace).unwrap();
        state.workspaces.delete(&workspace.id).unwrap();
        assert!(state.workspaces.load_all().unwrap().is_empty());
        assert!(state.registry.load_all().unwrap()[0]
            .mcp_policy
            .rules
            .is_empty());
    }

    #[test]
    fn unscoped_workspace_update_can_bind_an_exact_policy_rule_id() {
        let storage = StorageRecord::new(
            "Workspace storage".into(),
            "local".into(),
            serde_json::json!({"root": "/tmp"}),
        );
        let workspace = WorkspaceRecord {
            policy_rule_id: None,
            ..make_workspace_record("legacy", &storage.id, "/workspace")
        };
        assert_eq!(
            resolve_legacy_policy_rule_id(&storage, &workspace),
            "workspace:legacy"
        );
    }

    #[test]
    fn legacy_workspace_source_rule_is_adopted_when_metadata_lacks_policy_id() {
        let mut storage = StorageRecord::new(
            "Workspace storage".into(),
            "local".into(),
            serde_json::json!({"root": "/tmp"}),
        );
        storage.mcp_policy.rules.push(McpPathRule {
            id: "ws:legacy-source".into(),
            prefix: "workspace".into(),
            access: McpAccessMode::ReadWrite,
            source: McpRuleSource::Workspace {
                workspace_id: "legacy-source".into(),
            },
            confirmation_rules: None,
        });
        let workspace = WorkspaceRecord {
            policy_rule_id: None,
            ..make_workspace_record("legacy-source", &storage.id, "/workspace")
        };
        assert_eq!(
            resolve_legacy_policy_rule_id(&storage, &workspace),
            "ws:legacy-source"
        );
    }

    #[test]
    fn legacy_workspace_adopts_manual_rule_and_preserves_prior_rule_id() {
        let mut storage = StorageRecord::new(
            "Workspace storage".into(),
            "local".into(),
            serde_json::json!({"root": "/tmp"}),
        );
        storage.mcp_policy.rules.push(McpPathRule {
            id: "migrated-0".into(),
            prefix: "workspace".into(),
            access: McpAccessMode::ReadOnly,
            source: McpRuleSource::Manual,
            confirmation_rules: None,
        });
        let workspace = WorkspaceRecord {
            policy_rule_id: None,
            ..make_workspace_record("legacy", &storage.id, "/workspace")
        };
        assert_eq!(
            resolve_legacy_policy_rule_id(&storage, &workspace),
            "migrated-0"
        );
        let prior = WorkspaceRecord {
            policy_rule_id: Some("ws:legacy".into()),
            ..workspace
        };
        assert_eq!(resolve_legacy_policy_rule_id(&storage, &prior), "ws:legacy");
    }

    #[tokio::test]
    async fn test_policy_rule_applied_and_removed() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);

        let ws_id = "ws-policy-test";
        let ws = WorkspaceRecord {
            policy_rule_id: Some("ws:ws-policy-test".into()),
            ..make_workspace_record(ws_id, &storage_id, "/policy-test")
        };

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
        assert_eq!(storage.mcp_policy.rules[0].id, "ws:ws-policy-test");
        assert_eq!(storage.revision, 2);

        remove_workspace_policy_rule(&storage_registry, &storage_id, &ws)
            .expect("remove policy rule");

        let storages = storage_registry.load_all().unwrap();
        let storage = storages.iter().find(|s| s.id == storage_id).unwrap();
        assert!(storage.mcp_policy.rules.is_empty());
        assert_eq!(storage.revision, 3);
    }

    #[test]
    fn workspace_policy_deletion_fails_closed_on_wrong_source_or_missing_rule() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, _op) = create_test_op();
        let (_workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = WorkspaceRecord {
            policy_rule_id: Some("ws:fail-closed".into()),
            ..make_workspace_record("fail-closed", &storage_id, "/protected")
        };
        storage_registry
            .with_locked_mutation(|storages| {
                storages[0].mcp_policy.rules.push(McpPathRule {
                    id: "ws:fail-closed".into(),
                    prefix: "protected".into(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                });
                Ok(())
            })
            .unwrap();
        assert!(remove_workspace_policy_rule(&storage_registry, &storage_id, &workspace).is_err());
        assert_eq!(
            storage_registry.load_all().unwrap()[0]
                .mcp_policy
                .rules
                .len(),
            1
        );
        storage_registry
            .with_locked_mutation(|storages| {
                storages[0].mcp_policy.rules.clear();
                Ok(())
            })
            .unwrap();
        assert!(remove_workspace_policy_rule(&storage_registry, &storage_id, &workspace).is_err());
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

    async fn prepare_checkpoint_workspace(
        op: &opendal::Operator,
        workspace_registry: &WorkspaceRegistry,
        storage_registry: &StorageRegistry,
        workspace: &WorkspaceRecord,
    ) -> StorageRecord {
        for relative in &workspace.memory_files {
            let path = join_path(&workspace.root_path, relative);
            if let Some(parent) = path.rsplit_once('/').map(|value| value.0) {
                infimount_core::operations::create_directory(op, parent)
                    .await
                    .unwrap();
            }
            infimount_core::operations::write_full(
                op,
                &path,
                format!("current:{relative}").as_bytes(),
            )
            .await
            .unwrap();
        }
        infimount_core::operations::create_directory(
            op,
            &join_path(&workspace.root_path, ".infimount/checkpoints"),
        )
        .await
        .unwrap();
        workspace_registry.create(workspace).unwrap();
        let mut mutations = Vec::new();
        apply_workspace_policy_rule(
            storage_registry,
            &workspace.storage_id,
            workspace,
            McpAccessMode::ReadWrite,
            &mut mutations,
        )
        .unwrap();
        storage_registry
            .load_all()
            .unwrap()
            .into_iter()
            .find(|storage| storage.id == workspace.storage_id)
            .unwrap()
    }

    #[tokio::test]
    async fn checkpoint_creation_writes_manifest_then_registry_metadata() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace =
            make_workspace_record("checkpoint-success", &storage_id, "/checkpoint-success");
        let storage =
            prepare_checkpoint_workspace(&op, &workspace_registry, &storage_registry, &workspace)
                .await;

        let checkpoint = create_checkpoint_transaction(
            &op,
            &workspace_registry,
            &storage,
            workspace.clone(),
            Some("Before refactor"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(checkpoint.memory_files.len(), workspace.memory_files.len());
        assert!(op
            .stat(&join_path(&workspace.root_path, &checkpoint.manifest_path))
            .await
            .is_ok());
        let persisted = workspace_registry
            .find_by_id(&workspace.id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.checkpoint_ids, vec![checkpoint.id]);
    }

    #[tokio::test]
    async fn checkpoint_creation_rolls_back_manifest_when_registry_update_fails() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record(
            "checkpoint-create-rollback",
            &storage_id,
            "/create-rollback",
        );
        let storage =
            prepare_checkpoint_workspace(&op, &workspace_registry, &storage_registry, &workspace)
                .await;

        let error = create_checkpoint_transaction(
            &op,
            &workspace_registry,
            &storage,
            workspace.clone(),
            None,
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INTERNAL);
        let entries = infimount_core::operations::list_entries(
            &op,
            &join_path(&workspace.root_path, ".infimount/checkpoints"),
        )
        .await
        .unwrap();
        assert!(
            entries.iter().all(|entry| entry.is_dir),
            "checkpoint manifest must be removed: {entries:?}"
        );
        assert!(workspace_registry
            .find_by_id(&workspace.id)
            .unwrap()
            .unwrap()
            .checkpoint_ids
            .is_empty());
    }

    #[tokio::test]
    async fn checkpoint_restore_rolls_back_files_after_injected_failure() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record(
            "checkpoint-restore-rollback",
            &storage_id,
            "/restore-rollback",
        );
        let storage =
            prepare_checkpoint_workspace(&op, &workspace_registry, &storage_registry, &workspace)
                .await;
        let checkpoint = create_checkpoint_transaction(
            &op,
            &workspace_registry,
            &storage,
            workspace.clone(),
            None,
            false,
        )
        .await
        .unwrap();
        let persisted = workspace_registry
            .find_by_id(&workspace.id)
            .unwrap()
            .unwrap();
        for relative in &workspace.memory_files {
            infimount_core::operations::write_full(
                &op,
                &join_path(&workspace.root_path, relative),
                format!("after:{relative}").as_bytes(),
            )
            .await
            .unwrap();
        }

        let error =
            restore_checkpoint_transaction(&op, &storage, &persisted, &checkpoint.id, Some(1))
                .await
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INTERNAL);
        for relative in &workspace.memory_files {
            let bytes = infimount_core::operations::read_full(
                &op,
                &join_path(&workspace.root_path, relative),
            )
            .await
            .unwrap();
            assert_eq!(bytes.to_vec(), format!("after:{relative}").into_bytes());
        }
    }

    #[tokio::test]
    async fn checkpoint_missing_workspace_and_checkpoint_fail_closed() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        assert_eq!(
            find_workspace_or_error(&workspace_registry, "missing")
                .unwrap_err()
                .code,
            McpErrorCode::ERR_STORAGE_NOT_FOUND
        );
        let workspace = make_workspace_record("checkpoint-missing", &storage_id, "/missing");
        let storage =
            prepare_checkpoint_workspace(&op, &workspace_registry, &storage_registry, &workspace)
                .await;
        let error = restore_checkpoint_transaction(
            &op,
            &storage,
            &workspace,
            "checkpoint-not-present",
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_STORAGE_NOT_FOUND);
    }

    #[tokio::test]
    async fn checkpoint_control_plane_ignores_mcp_policy_but_honors_storage_read_only() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record("checkpoint-denied", &storage_id, "/denied");
        for relative in &workspace.memory_files {
            let path = join_path(&workspace.root_path, relative);
            infimount_core::operations::create_directory(&op, path.rsplit_once('/').unwrap().0)
                .await
                .unwrap();
            infimount_core::operations::write_full(&op, &path, b"safe")
                .await
                .unwrap();
        }
        workspace_registry.create(&workspace).unwrap();
        let mut storage = storage_registry.load_all().unwrap().remove(0);
        let checkpoint = create_checkpoint_transaction(
            &op,
            &workspace_registry,
            &storage,
            workspace.clone(),
            None,
            false,
        )
        .await
        .expect("desktop checkpoint management must not require MCP exposure or policy");
        assert_eq!(checkpoint.workspace_id, workspace.id);

        storage.read_only = true;
        let error = validate_checkpoint_context(&op, &storage, &workspace, true).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
    }

    #[tokio::test]
    async fn checkpoint_bounds_reject_oversized_file_and_full_index() {
        let temp_config = tempdir().unwrap();
        let (temp_storage, op) = create_test_op();
        let (workspace_registry, storage_registry) = create_test_registries(&temp_config);
        let storage_id = create_test_storage(&storage_registry, &temp_storage);
        let workspace = make_workspace_record("checkpoint-bounds", &storage_id, "/bounds");
        let storage =
            prepare_checkpoint_workspace(&op, &workspace_registry, &storage_registry, &workspace)
                .await;
        infimount_core::operations::write_full(
            &op,
            &join_path(&workspace.root_path, &workspace.memory_files[0]),
            &vec![b'x'; usize::try_from(MAX_CHECKPOINT_FILE_BYTES).unwrap() + 1],
        )
        .await
        .unwrap();
        let error = create_checkpoint_transaction(
            &op,
            &workspace_registry,
            &storage,
            workspace.clone(),
            None,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_PATH);

        let mut full = workspace;
        full.checkpoint_ids = (0..MAX_CHECKPOINT_IDS)
            .map(|index| format!("checkpoint-{index}"))
            .collect();
        let error =
            create_checkpoint_transaction(&op, &workspace_registry, &storage, full, None, false)
                .await
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_PATH);
        assert!(
            validate_checkpoint_label(Some(&"x".repeat(MAX_CHECKPOINT_LABEL_BYTES + 1))).is_err()
        );
    }
}
