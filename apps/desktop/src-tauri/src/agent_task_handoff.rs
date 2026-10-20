use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use infimount_core::agent_task_io::hash_agent_task_file;
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASK_BRIEF_FILE,
    AGENT_TASK_INPUTS_DIR, AGENT_TASK_MANIFEST_FILE, AGENT_TASK_OUTPUTS_DIR,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use infimount_mcp::AgentTaskScope;
use uuid::Uuid;

use crate::state::AppState;

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

/// A validated task and its task-only MCP boundary. This is not an agent
/// execution grant or publication authorization. The original storage source
/// paths and host workspace path are never returned to the client adapter.
pub(crate) struct PreparedAgentTaskHandoff {
    pub(crate) workspace_name: String,
    pub(crate) task_root: String,
    pub(crate) scope: AgentTaskScope,
    pub(crate) mcp_task_path: String,
    pub(crate) mcp_outputs_path: String,
}

/// Shared preflight for an Agent Task handoff. Keep the same v0.8.1 order:
/// validate identity and policy, verify task manifest and all prepared bytes,
/// then re-check both host-path confinement and live policy binding.
/// This function must never expand an MCP permission or mutate task inputs.
pub(crate) async fn prepare_agent_task_handoff(
    state: &AppState,
    client_name: &str,
    workspace_id: &str,
    task_id: &str,
) -> Result<PreparedAgentTaskHandoff, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config("Agent Task handoff is unavailable while Infimount is degraded".into())
    })?;

    Uuid::parse_str(task_id)
        .map_err(|_| CoreError::Config("Agent Task handoff requires a valid task id".into()))?;
    Uuid::parse_str(workspace_id).map_err(|_| {
        CoreError::Config("Agent Task handoff requires a valid workspace id".into())
    })?;

    let workspace = state
        .workspaces
        .find_by_id(workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }
    if workspace.access_profile != "read_write" {
        return Err(CoreError::Config(format!(
            "{client_name} handoff requires a read-write Agent Workspace"
        )));
    }
    let policy_rule_id = workspace.policy_rule_id.clone().ok_or_else(|| {
        CoreError::Config(format!(
            "{client_name} handoff requires the Agent Workspace MCP policy to be applied"
        ))
    })?;

    let storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(|_| {
            CoreError::Config("Agent Task workspace storage could not be validated".into())
        })?;
    if !storage.enabled || storage.read_only {
        return Err(CoreError::Config(format!(
            "{client_name} handoff requires an enabled writable workspace storage"
        )));
    }
    if !storage.mcp_exposed {
        return Err(CoreError::Config(format!(
            "{client_name} handoff requires the Agent Workspace storage to be exposed to MCP"
        )));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(kind, SourceKind::Local) {
        return Err(CoreError::Config(format!(
            "{client_name} handoff currently requires a Local Filesystem Agent Workspace"
        )));
    }

    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".into()))?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let task_root = agent_task_root(task_id)?;
    let workspace_task_path = join_path(&workspace.root_path, &task_root);
    let outputs_path = join_path(&workspace_task_path, AGENT_TASK_OUTPUTS_DIR);
    let scope = infimount_mcp::AgentTaskScope::new(
        task_id.to_owned(),
        storage.id.clone(),
        storage.name.clone(),
        workspace.id.clone(),
        policy_rule_id,
        namespace,
        workspace.root_path.clone(),
        workspace_task_path.clone(),
        outputs_path.clone(),
    )
    .map_err(|_| {
        CoreError::Config("Agent Workspace MCP policy does not match this prepared task".into())
    })?;
    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| {
            CoreError::Config(
                "Agent Workspace MCP policy no longer grants this workspace read-write access"
                    .into(),
            )
        })?;

    validate_local_path(&storage, &workspace.root_path)?;
    validate_local_path(&storage, &workspace_task_path)?;

    let op = state.operator_for_storage_id(&storage.id)?;
    require_directory(&op, &workspace_task_path, "Agent Task package").await?;

    let manifest_path = join_path(&workspace_task_path, AGENT_TASK_MANIFEST_FILE);
    let brief_path = join_path(&workspace_task_path, AGENT_TASK_BRIEF_FILE);
    for path in [&manifest_path, &brief_path, &outputs_path] {
        validate_local_path(&storage, path)?;
    }
    require_file(&op, &brief_path, "Agent Task brief").await?;
    require_directory(&op, &outputs_path, "Agent Task outputs directory").await?;

    let manifest_metadata = op.stat(&manifest_path).await?;
    if !manifest_metadata.is_file() {
        return Err(CoreError::Config(
            "Agent Task manifest is missing or is not a file".into(),
        ));
    }
    if manifest_metadata.content_length() > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest_bytes = op.read(&manifest_path).await?.to_vec();
    if manifest_bytes.len() as u64 > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest: AgentTaskManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_agent_task_manifest(&manifest)?;
    if manifest.task_id != task_id
        || manifest.workspace_id != workspace_id
        || manifest.task_root != task_root
    {
        return Err(CoreError::Config(
            "Agent Task manifest does not match the requested workspace task".into(),
        ));
    }

    verify_prepared_inputs(&op, &storage, &workspace_task_path, &manifest).await?;

    // Re-run confinement and policy binding immediately before handing the task to an
    // external client. No adapter receives the workspace host path.
    validate_local_path(&storage, &workspace_task_path)?;
    scope
        .validate_current_binding(&state.registry)
        .map_err(|_| {
            CoreError::Config(format!(
                "Agent Workspace MCP policy changed before {client_name} handoff; review it and retry"
            ))
        })?;

    let mcp_task_path = format!("/{}/{}", storage.name, scope.task_prefix.trim_matches('/'));
    let mcp_outputs_path = format!(
        "/{}/{}",
        storage.name,
        scope.outputs_prefix.trim_matches('/')
    );

    Ok(PreparedAgentTaskHandoff {
        workspace_name: workspace.name,
        task_root,
        scope,
        mcp_task_path,
        mcp_outputs_path,
    })
}

async fn verify_prepared_inputs(
    op: &opendal::Operator,
    storage: &infimount_mcp::registry::StorageRecord,
    workspace_task_path: &str,
    manifest: &AgentTaskManifest,
) -> Result<(), CoreError> {
    let inputs_root = join_path(workspace_task_path, AGENT_TASK_INPUTS_DIR);
    validate_local_path(storage, &inputs_root)?;
    require_directory(op, &inputs_root, "Agent Task inputs directory").await?;

    let entries = infimount_core::operations::list_entries_recursive(op, &inputs_root)
        .await
        .map_err(|_| {
            CoreError::Config("Agent Task prepared input set could not be verified".into())
        })?;
    if entries.len() >= infimount_core::models::MAX_RECURSIVE_ITEMS as usize {
        return Err(CoreError::Config(
            "Agent Task prepared input set is too large to verify without truncation".into(),
        ));
    }
    let actual_files = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.path.trim_matches('/').to_string())
        .collect::<HashSet<_>>();
    let expected_files = expected_prepared_input_paths(workspace_task_path, manifest);

    if actual_files != expected_files {
        return Err(CoreError::Config(
            "Agent Task prepared input set changed after preparation".into(),
        ));
    }

    let mut inputs = manifest.inputs.iter().collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));
    for input in inputs {
        let path = join_path(workspace_task_path, &input.task_path);
        validate_local_path(storage, &path)?;
        let digest = hash_agent_task_file(op, &path).await.map_err(|_| {
            CoreError::Config("Agent Task prepared input bytes could not be verified".into())
        })?;
        if digest.byte_size != input.byte_size || digest.sha256 != input.sha256 {
            return Err(CoreError::Config(
                "Agent Task prepared input bytes changed after preparation".into(),
            ));
        }
    }

    Ok(())
}

pub(crate) fn expected_prepared_input_paths(
    workspace_task_path: &str,
    manifest: &AgentTaskManifest,
) -> HashSet<String> {
    manifest
        .inputs
        .iter()
        .map(|input| {
            join_path(workspace_task_path, &input.task_path)
                .trim_matches('/')
                .to_string()
        })
        .collect()
}

async fn require_directory(
    op: &opendal::Operator,
    path: &str,
    label: &str,
) -> Result<(), CoreError> {
    let metadata = op
        .stat(path)
        .await
        .map_err(|_| CoreError::Config(format!("{label} could not be read")))?;
    if !metadata.is_dir() {
        return Err(CoreError::Config(format!("{label} is not a directory")));
    }
    Ok(())
}

async fn require_file(op: &opendal::Operator, path: &str, label: &str) -> Result<(), CoreError> {
    let metadata = op
        .stat(path)
        .await
        .map_err(|_| CoreError::Config(format!("{label} could not be read")))?;
    if !metadata.is_file() {
        return Err(CoreError::Config(format!("{label} is not a file")));
    }
    Ok(())
}

fn validate_local_path(
    storage: &infimount_mcp::registry::StorageRecord,
    path: &str,
) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path)
        .map_err(|_| CoreError::Config("Agent Task local path confinement check failed".into()))
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

pub(crate) fn create_neutral_handoff_directory(client_name: &str) -> Result<PathBuf, CoreError> {
    // Static adapter names become filesystem components. Never accept path separators
    // or attacker-controlled free-form labels from future integrations.
    if client_name.is_empty()
        || client_name.len() > 32
        || !client_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(CoreError::Config("invalid Agent Task client name".into()));
    }
    let path = std::env::temp_dir().join(format!(
        "infimount-{}-{}",
        client_name.to_ascii_lowercase(),
        Uuid::new_v4()
    ));
    fs::create_dir(&path).map_err(|_| {
        CoreError::Config(format!(
            "failed to create the {client_name} handoff directory"
        ))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions_result = fs::metadata(&path).and_then(|metadata| {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&path, permissions)
        });
        if permissions_result.is_err() {
            cleanup_neutral_directory(&path);
            return Err(CoreError::Config(format!(
                "failed to secure the {client_name} handoff directory"
            )));
        }
    }

    Ok(path)
}

pub(crate) fn cleanup_neutral_directory(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

pub(crate) fn agent_task_prompt(
    workspace_name: &str,
    task_path: &str,
    outputs_path: &str,
) -> String {
    format!(
        "Complete the prepared Infimount Agent Task using only the required task-scoped Infimount MCP server for task file access. Work only in Agent Workspace {workspace_name:?} at MCP path {task_path:?}. Read TASK.md through Infimount and treat inputs/ plus task metadata as immutable. Infimount enforces writes only under {outputs_path:?}; use write_file (with create_parents when needed) for deliverables. Stop when the deliverables are ready for review in Infimount. Do not publish outputs. Do not search for or use host filesystem paths to bypass the task boundary."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_client_directory_labels() {
        for label in [
            "",
            "../codex",
            "claude/code",
            "codex\\\\escape",
            "a b",
            "a.b",
        ] {
            assert!(create_neutral_handoff_directory(label).is_err(), "{label}");
        }
    }

    #[test]
    fn codex_handoff_directory_remains_private_and_compatible() {
        let root = create_neutral_handoff_directory("Codex").unwrap();
        let name = root.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("infimount-codex-"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        cleanup_neutral_directory(&root);
        assert!(!root.exists());
    }

    #[test]
    fn task_prompt_keeps_data_and_publication_boundaries() {
        let prompt = agent_task_prompt("Workspace", "/scope/task", "/scope/task/outputs");
        assert!(prompt.contains("task-scoped Infimount MCP server"));
        assert!(prompt.contains("treat inputs/ plus task metadata as immutable"));
        assert!(prompt.contains("Infimount enforces writes only under"));
        assert!(prompt.contains("Do not publish outputs"));
        assert!(!prompt.contains("/home/"));
    }
}
