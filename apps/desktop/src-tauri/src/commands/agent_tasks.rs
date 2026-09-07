use std::collections::HashSet;

use infimount_core::agent_task_io::{copy_agent_task_plan, hash_agent_task_file};
use infimount_core::agent_tasks::{
    agent_task_root, render_agent_task_markdown, validate_agent_task_brief, AgentTaskBrief,
    AgentTaskInput, AgentTaskManifest, AGENT_TASKS_DIR, AGENT_TASK_BRIEF_FILE,
    AGENT_TASK_INPUTS_DIR, AGENT_TASK_MANIFEST_FILE, AGENT_TASK_OUTPUTS_DIR,
    AGENT_TASK_SCHEMA_VERSION, MAX_AGENT_TASK_INPUTS, MAX_AGENT_TASK_PREPARED_BYTES,
    MAX_AGENT_TASK_SELECTIONS,
};
use infimount_core::workspaces::{workspace_schema_supported, WorkspaceRecord};
use infimount_core::{operations, CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_SOURCE_PATH_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskPreparationRequest {
    pub source_storage_id: String,
    pub source_paths: Vec<String>,
    pub workspace_id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub requested_outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPreflightOutput {
    pub workspace_id: String,
    pub workspace_name: String,
    pub selected_items: usize,
    pub file_count: usize,
    pub directory_count: usize,
    pub total_bytes: u64,
    pub source_mcp_exposed: bool,
    pub workspace_mcp_exposed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareAgentTaskOutput {
    pub task_id: String,
    pub task_root: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub prepared_files: usize,
    pub total_bytes: u64,
    pub source_mcp_exposed: bool,
    pub workspace_mcp_exposed: bool,
}

struct AgentTaskContext {
    workspace: WorkspaceRecord,
    source_storage: StorageRecord,
    workspace_storage: StorageRecord,
    source_op: opendal::Operator,
    workspace_op: opendal::Operator,
}

struct PlannedTask {
    plan: operations::TransferPlan,
    file_count: usize,
    directory_count: usize,
    total_bytes: u64,
}

#[tauri::command]
pub async fn preflight_agent_task(
    state: State<'_, AppState>,
    request: AgentTaskPreparationRequest,
) -> Result<AgentTaskPreflightOutput, CoreError> {
    state.require_operational().map_err(mcp_to_core)?;
    let (brief, source_paths) = validate_request(&request)?;
    let context = load_context(&state, &request)?;

    let probe_id = Uuid::new_v4();
    let probe_root = join_path(
        &context.workspace.root_path,
        &format!("{AGENT_TASKS_DIR}/.preflight-{probe_id}"),
    );
    let target_inputs = join_path(&probe_root, AGENT_TASK_INPUTS_DIR);
    let planned = plan_task(&context, &source_paths, &target_inputs).await?;

    // Keep validation of the brief in this command even though validate_request
    // already performs it. It documents the server-side preflight contract and
    // prevents a later request-normalization refactor from accidentally dropping it.
    validate_agent_task_brief(&brief)?;

    Ok(AgentTaskPreflightOutput {
        workspace_id: context.workspace.id.clone(),
        workspace_name: context.workspace.name.clone(),
        selected_items: source_paths.len(),
        file_count: planned.file_count,
        directory_count: planned.directory_count,
        total_bytes: planned.total_bytes,
        source_mcp_exposed: context.source_storage.mcp_exposed,
        workspace_mcp_exposed: context.workspace_storage.mcp_exposed,
        warnings: exposure_warnings(&context),
    })
}

#[tauri::command]
pub async fn prepare_agent_task(
    state: State<'_, AppState>,
    request: AgentTaskPreparationRequest,
) -> Result<PrepareAgentTaskOutput, CoreError> {
    state.require_operational().map_err(mcp_to_core)?;
    let (brief, source_paths) = validate_request(&request)?;

    // Serialize workspace create/update/delete against the complete preparation
    // transaction. File browsing and ordinary storage operations are unaffected.
    let _workspace_transaction = state.workspaces.acquire_mutation_lock()?;
    let context = load_context(&state, &request)?;

    let task_id = Uuid::new_v4().to_string();
    let task_root = agent_task_root(&task_id)?;
    let final_root = join_path(&context.workspace.root_path, &task_root);
    let staging_relative = format!("{AGENT_TASKS_DIR}/.preparing-{task_id}");
    let staging_root = join_path(&context.workspace.root_path, &staging_relative);
    let staging_inputs = join_path(&staging_root, AGENT_TASK_INPUTS_DIR);
    let staging_outputs = join_path(&staging_root, AGENT_TASK_OUTPUTS_DIR);
    let tasks_root = join_path(&context.workspace.root_path, AGENT_TASKS_DIR);

    validate_local_path(&context.workspace_storage, &tasks_root)?;
    validate_local_path(&context.workspace_storage, &staging_root)?;
    validate_local_path(&context.workspace_storage, &final_root)?;

    ensure_directory(
        &context.workspace_op,
        &context.workspace_storage,
        &tasks_root,
    )
    .await?;
    require_missing(&context.workspace_op, &staging_root).await?;
    require_missing(&context.workspace_op, &final_root).await?;
    ensure_directory(
        &context.workspace_op,
        &context.workspace_storage,
        &staging_root,
    )
    .await?;

    let result = async {
        ensure_directory(
            &context.workspace_op,
            &context.workspace_storage,
            &staging_inputs,
        )
        .await?;
        ensure_directory(
            &context.workspace_op,
            &context.workspace_storage,
            &staging_outputs,
        )
        .await?;

        // Re-plan after the destination hierarchy exists, then execute exactly
        // those validated entries. Do not recursively enumerate the selected
        // source again after the authoritative file-count/byte-limit checks.
        let planned = plan_task(&context, &source_paths, &staging_inputs).await?;
        for entry in &planned.plan.entries {
            validate_local_path(&context.source_storage, &entry.source_path)?;
            validate_local_path(&context.workspace_storage, &entry.destination_path)?;
        }
        copy_agent_task_plan(&context.source_op, &context.workspace_op, &planned.plan).await?;

        let mut inputs = Vec::with_capacity(planned.file_count);
        let mut file_entries = planned
            .plan
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .collect::<Vec<_>>();
        file_entries.sort_by(|left, right| left.destination_path.cmp(&right.destination_path));

        for entry in file_entries {
            validate_local_path(&context.workspace_storage, &entry.destination_path)?;
            let task_path = relative_to_root(&staging_root, &entry.destination_path)?;
            let digest =
                hash_agent_task_file(&context.workspace_op, &entry.destination_path).await?;
            if digest.byte_size != entry.size {
                return Err(CoreError::Config(
                    "Agent Task prepared bytes no longer match the validated plan".to_string(),
                ));
            }
            inputs.push(AgentTaskInput {
                task_path,
                byte_size: digest.byte_size,
                sha256: digest.sha256,
            });
        }

        let prepared_bytes = inputs
            .iter()
            .fold(0_u64, |total, input| total.saturating_add(input.byte_size));
        if inputs.len() != planned.file_count || prepared_bytes != planned.total_bytes {
            return Err(CoreError::Config(
                "Agent Task prepared snapshot no longer matches the validated plan".to_string(),
            ));
        }

        let manifest = AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: task_id.clone(),
            title: brief.title.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            workspace_id: context.workspace.id.clone(),
            task_root: task_root.clone(),
            outputs_directory: AGENT_TASK_OUTPUTS_DIR.to_string(),
            inputs,
        };
        let markdown = render_agent_task_markdown(&manifest, &brief)?;
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        manifest_bytes.push(b'\n');

        let manifest_path = join_path(&staging_root, AGENT_TASK_MANIFEST_FILE);
        let brief_path = join_path(&staging_root, AGENT_TASK_BRIEF_FILE);
        validate_local_path(&context.workspace_storage, &manifest_path)?;
        validate_local_path(&context.workspace_storage, &brief_path)?;
        operations::write_full(&context.workspace_op, &manifest_path, &manifest_bytes).await?;
        operations::write_full(&context.workspace_op, &brief_path, markdown.as_bytes()).await?;

        // Re-check the local path just before the package commit. This preserves
        // the same accepted check-then-OpenDAL race model as the v0.8 MCP path
        // guard while refusing persistent symlink/reparse components.
        validate_local_path(&context.workspace_storage, &staging_root)?;
        validate_local_path(&context.workspace_storage, &final_root)?;
        require_missing(&context.workspace_op, &final_root).await?;

        // Match the directory-rename convention used by the existing transfer
        // transaction implementation: normalized paths without trailing slash.
        context
            .workspace_op
            .rename(
                staging_root.trim_end_matches('/'),
                final_root.trim_end_matches('/'),
            )
            .await?;

        Ok::<_, CoreError>(PrepareAgentTaskOutput {
            task_id: task_id.clone(),
            task_root: task_root.clone(),
            workspace_id: context.workspace.id.clone(),
            workspace_name: context.workspace.name.clone(),
            prepared_files: manifest.inputs.len(),
            total_bytes: prepared_bytes,
            source_mcp_exposed: context.source_storage.mcp_exposed,
            workspace_mcp_exposed: context.workspace_storage.mcp_exposed,
        })
    }
    .await;

    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            if cleanup_created_task(&context.workspace_op, &staging_root).await {
                Err(error)
            } else {
                Err(CoreError::TransferCleanupRequired)
            }
        }
    }
}

fn validate_request(
    request: &AgentTaskPreparationRequest,
) -> Result<(AgentTaskBrief, Vec<String>), CoreError> {
    if request.source_storage_id.trim().is_empty() || request.workspace_id.trim().is_empty() {
        return Err(CoreError::Config(
            "Agent Task source storage and workspace are required".to_string(),
        ));
    }
    if request.source_paths.is_empty() || request.source_paths.len() > MAX_AGENT_TASK_SELECTIONS {
        return Err(CoreError::Config(format!(
            "Agent Task must select between 1 and {MAX_AGENT_TASK_SELECTIONS} source items"
        )));
    }

    let brief = AgentTaskBrief {
        title: request.title.clone(),
        objective: request.objective.clone(),
        requested_outputs: request.requested_outputs.clone(),
    };
    validate_agent_task_brief(&brief)?;
    let paths = normalize_source_paths(&request.source_paths)?;
    Ok((brief, paths))
}

fn normalize_source_paths(paths: &[String]) -> Result<Vec<String>, CoreError> {
    let mut normalized = Vec::with_capacity(paths.len());
    let mut seen = HashSet::with_capacity(paths.len());
    for raw in paths {
        let path = raw.trim().trim_matches('/');
        if path.is_empty() || path.len() > MAX_SOURCE_PATH_BYTES || path.contains('\\') {
            return Err(CoreError::Config(
                "Agent Task source paths must be non-empty bounded forward-slash paths".to_string(),
            ));
        }
        if path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(CoreError::Config(
                "Agent Task source path contains an invalid segment".to_string(),
            ));
        }
        if !seen.insert(path.to_string()) {
            return Err(CoreError::Config(
                "Agent Task source selection contains duplicate paths".to_string(),
            ));
        }
        normalized.push(path.to_string());
    }

    normalized.sort();
    for (index, path) in normalized.iter().enumerate() {
        for other in normalized.iter().skip(index + 1) {
            if other.starts_with(&format!("{path}/")) {
                return Err(CoreError::Config(
                    "Agent Task source selections must not overlap".to_string(),
                ));
            }
        }
    }
    Ok(normalized)
}

fn load_context(
    state: &AppState,
    request: &AgentTaskPreparationRequest,
) -> Result<AgentTaskContext, CoreError> {
    let workspace = state
        .workspaces
        .find_by_id(&request.workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".to_string()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".to_string(),
        ));
    }
    if workspace.access_profile != "read_write" {
        return Err(CoreError::Config(
            "Agent Tasks require a read-write Agent Workspace so the agent can create outputs"
                .to_string(),
        ));
    }

    let source_storage = state
        .find_storage_by_id(&request.source_storage_id)
        .map_err(mcp_to_core)?;
    let workspace_storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(mcp_to_core)?;
    if !source_storage.enabled || !workspace_storage.enabled {
        return Err(CoreError::Config(
            "Agent Task source and workspace storages must be enabled".to_string(),
        ));
    }
    if workspace_storage.read_only {
        return Err(CoreError::Config(
            "Agent Task workspace storage is configured read-only".to_string(),
        ));
    }
    let workspace_kind = workspace_storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| {
            CoreError::Config("Agent Task workspace backend is unsupported".to_string())
        })?;
    if !matches!(workspace_kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Tasks currently require an Agent Workspace on Local Filesystem storage"
                .to_string(),
        ));
    }

    let namespace =
        infimount_mcp::storage_namespace::storage_namespace_fingerprint(&workspace_storage)
            .map_err(|_| {
                CoreError::Config("failed to verify Agent Task workspace identity".to_string())
            })?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".to_string(),
        ));
    }

    let source_op = state.operator_for_storage_id(&source_storage.id)?;
    let workspace_op = state.operator_for_storage_id(&workspace_storage.id)?;
    let source_caps = source_op.info().capability();
    if !source_caps.stat || !source_caps.read || !source_caps.list {
        return Err(CoreError::Config(
            "Agent Task source storage lacks required read/list capabilities".to_string(),
        ));
    }
    let workspace_caps = workspace_op.info().capability();
    if !workspace_caps.stat
        || !workspace_caps.read
        || !workspace_caps.write
        || !workspace_caps.create_dir
        || !workspace_caps.list
        || !workspace_caps.delete
        || !workspace_caps.rename
    {
        return Err(CoreError::Config(
            "Agent Task local workspace lacks required package capabilities".to_string(),
        ));
    }

    validate_local_path(&workspace_storage, &workspace.root_path)?;
    Ok(AgentTaskContext {
        workspace,
        source_storage,
        workspace_storage,
        source_op,
        workspace_op,
    })
}

async fn plan_task(
    context: &AgentTaskContext,
    source_paths: &[String],
    target_inputs: &str,
) -> Result<PlannedTask, CoreError> {
    for path in source_paths {
        validate_local_path(&context.source_storage, path)?;
        let destination = operations::transfer_destination_path(path, target_inputs);
        let relation = infimount_mcp::storage_namespace::transfer_namespace_relation(
            &context.source_storage,
            path,
            &context.workspace_storage,
            &destination,
        )
        .map_err(|_| CoreError::Config("failed to compare Agent Task namespaces".to_string()))?;
        if infimount_mcp::storage_namespace::transfer_has_namespace_conflict(&relation) {
            return Err(CoreError::Config(
                "Agent Task destination overlaps a selected source path".to_string(),
            ));
        }
    }

    validate_local_path(&context.workspace_storage, target_inputs)?;
    let plan = operations::plan_transfer_entries(
        &context.source_op,
        &context.workspace_op,
        source_paths.to_vec(),
        target_inputs,
        operations::TransferOperation::Copy,
        context.source_storage.id == context.workspace_storage.id,
        operations::TransferConflictPolicy::Fail,
    )
    .await?;

    let file_count = plan.entries.iter().filter(|entry| !entry.is_dir).count();
    let directory_count = plan.entries.len().saturating_sub(file_count);
    if file_count == 0 {
        return Err(CoreError::Config(
            "Agent Task selection contains no files".to_string(),
        ));
    }
    if file_count > MAX_AGENT_TASK_INPUTS {
        return Err(CoreError::Config(format!(
            "Agent Task selection contains more than {MAX_AGENT_TASK_INPUTS} files"
        )));
    }
    if plan.summary.total_bytes > MAX_AGENT_TASK_PREPARED_BYTES {
        return Err(CoreError::Config(format!(
            "Agent Task selection exceeds the {} byte preparation limit",
            MAX_AGENT_TASK_PREPARED_BYTES
        )));
    }

    let mut destinations = HashSet::with_capacity(plan.entries.len());
    for entry in &plan.entries {
        validate_local_path(&context.source_storage, &entry.source_path)?;
        validate_local_path(&context.workspace_storage, &entry.destination_path)?;
        let portable = entry.destination_path.to_lowercase();
        if !destinations.insert(portable) {
            return Err(CoreError::Config(
                "Agent Task prepared paths collide case-insensitively".to_string(),
            ));
        }
        if !matches!(entry.action, operations::TransferPlanAction::Create) {
            return Err(CoreError::Config(
                "Agent Task preparation destination is not empty".to_string(),
            ));
        }
    }

    Ok(PlannedTask {
        total_bytes: plan.summary.total_bytes,
        plan,
        file_count,
        directory_count,
    })
}

fn exposure_warnings(context: &AgentTaskContext) -> Vec<String> {
    let mut warnings = Vec::new();
    if context.source_storage.mcp_exposed {
        warnings.push(
            "The source storage already has independent MCP exposure. This task does not add that access, but existing policy may still allow the client to reach it."
                .to_string(),
        );
    }
    if !context.workspace_storage.mcp_exposed {
        warnings.push(
            "The Agent Workspace storage is not currently exposed to MCP. Prepare can continue, but agent handoff will require enabling its existing workspace-scoped access."
                .to_string(),
        );
    }
    warnings
}

async fn ensure_directory(
    op: &opendal::Operator,
    storage: &StorageRecord,
    path: &str,
) -> Result<(), CoreError> {
    validate_local_path(storage, path)?;
    match op.stat(path).await {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => {
            return Err(CoreError::Config(
                "Agent Task package path already exists as a file".to_string(),
            ))
        }
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    operations::create_directory(op, path).await?;
    validate_local_path(storage, path)?;
    Ok(())
}

async fn require_missing(op: &opendal::Operator, path: &str) -> Result<(), CoreError> {
    match op.stat(path).await {
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(CoreError::Config(
            "Agent Task package path already exists; prepare again".to_string(),
        )),
    }
}

async fn cleanup_created_task(op: &opendal::Operator, staging_root: &str) -> bool {
    match op.stat(staging_root).await {
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => true,
        Err(_) => false,
        Ok(_) => operations::delete(op, staging_root).await.is_ok(),
    }
}

fn validate_local_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path).map_err(|_| {
        CoreError::Config("Agent Task local path confinement check failed".to_string())
    })
}

fn relative_to_root(root: &str, path: &str) -> Result<String, CoreError> {
    let root = root.trim_matches('/');
    let path = path.trim_matches('/');
    let prefix = format!("{root}/");
    path.strip_prefix(&prefix)
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CoreError::Config("Agent Task destination escaped its package root".to_string())
        })
}

fn join_path(root: &str, relative: &str) -> String {
    let root = root.trim().trim_matches('/');
    let relative = relative.trim().trim_matches('/');
    match (root.is_empty(), relative.is_empty()) {
        (true, true) => String::new(),
        (true, false) => relative.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}/{relative}"),
    }
}

fn mcp_to_core(_error: infimount_mcp::errors::McpError) -> CoreError {
    CoreError::Config("Agent Task request could not be validated".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_selection_rejects_duplicates_and_overlap() {
        assert!(normalize_source_paths(&["a.txt".into(), "a.txt".into()]).is_err());
        assert!(normalize_source_paths(&["folder".into(), "folder/file.txt".into()]).is_err());
        assert!(normalize_source_paths(&["folder/../secret".into()]).is_err());
    }

    #[test]
    fn source_selection_is_sorted_and_normalized() {
        let paths = normalize_source_paths(&["/z.txt/".into(), "a.txt".into()]).unwrap();
        assert_eq!(paths, vec!["a.txt", "z.txt"]);
    }

    #[test]
    fn task_relative_path_cannot_escape_staging_root() {
        assert_eq!(
            relative_to_root(
                "workspace/tasks/.preparing-id",
                "workspace/tasks/.preparing-id/inputs/a.txt"
            )
            .unwrap(),
            "inputs/a.txt"
        );
        assert!(relative_to_root("workspace/tasks/.preparing-id", "workspace/other.txt").is_err());
    }
}
