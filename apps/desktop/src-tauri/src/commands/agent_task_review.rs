use infimount_core::agent_task_io::hash_agent_task_file;
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASK_MANIFEST_FILE,
    AGENT_TASK_OUTPUTS_DIR,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{operations, CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_TASK_OUTPUT_REVIEW_FILES: usize = 1_000;
const MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_AGENT_TASK_OUTPUT_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_AGENT_TASK_OUTPUT_PREVIEW_FILES: usize = 20;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskOutputReviewRequest {
    pub workspace_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputReviewFile {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub preview: Option<String>,
    pub preview_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputReviewOutput {
    pub workspace_id: String,
    pub workspace_name: String,
    pub task_id: String,
    pub task_root: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<AgentTaskOutputReviewFile>,
}

struct ReviewContext {
    workspace: infimount_core::workspaces::WorkspaceRecord,
    storage: StorageRecord,
    op: opendal::Operator,
    task_root: String,
    workspace_task_path: String,
}

#[tauri::command]
pub async fn review_agent_task_outputs(
    state: State<'_, AppState>,
    request: AgentTaskOutputReviewRequest,
) -> Result<AgentTaskOutputReviewOutput, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Agent Task output review is unavailable while Infimount is degraded".into(),
        )
    })?;

    let context = load_review_context(&state, &request)?;
    validate_task_manifest(&context, &request).await?;

    let outputs_path = join_path(&context.workspace_task_path, AGENT_TASK_OUTPUTS_DIR);
    validate_local_path(&context.storage, &outputs_path)?;
    require_directory(&context.op, &outputs_path, "Agent Task outputs directory").await?;

    let entries = operations::list_entries_recursive(&context.op, &outputs_path)
        .await
        .map_err(|_| CoreError::Config("Agent Task outputs could not be enumerated".into()))?;
    if entries.len() >= infimount_core::models::MAX_RECURSIVE_ITEMS as usize {
        return Err(CoreError::Config(
            "Agent Task outputs are too large to review without truncation".into(),
        ));
    }

    let mut file_entries = entries
        .into_iter()
        .filter(|entry| !entry.is_dir)
        .collect::<Vec<_>>();
    file_entries.sort_by(|left, right| left.path.cmp(&right.path));
    if file_entries.len() > MAX_AGENT_TASK_OUTPUT_REVIEW_FILES {
        return Err(CoreError::Config(format!(
            "Agent Task output review supports at most {MAX_AGENT_TASK_OUTPUT_REVIEW_FILES} files"
        )));
    }
    let listed_bytes = file_entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.size)
            .ok_or_else(|| CoreError::Config("Agent Task output byte count overflowed".into()))
    })?;
    if listed_bytes > MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES {
        return Err(CoreError::Config(
            "Agent Task outputs exceed the 2 GiB review limit".into(),
        ));
    }

    let mut files = Vec::with_capacity(file_entries.len());
    let mut total_bytes = 0_u64;
    for (index, entry) in file_entries.into_iter().enumerate() {
        validate_local_path(&context.storage, &entry.path)?;
        let task_path = relative_to_root(&context.workspace_task_path, &entry.path)?;
        if !task_path.starts_with("outputs/") || task_path.len() <= "outputs/".len() {
            return Err(CoreError::Config(
                "Agent Task output escaped the outputs directory".into(),
            ));
        }

        let digest = hash_agent_task_file(&context.op, &entry.path)
            .await
            .map_err(|_| CoreError::Config("Agent Task output bytes could not be hashed".into()))?;
        total_bytes = total_bytes
            .checked_add(digest.byte_size)
            .ok_or_else(|| CoreError::Config("Agent Task output byte count overflowed".into()))?;
        if total_bytes > MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES {
            return Err(CoreError::Config(
                "Agent Task outputs changed beyond the 2 GiB review limit".into(),
            ));
        }

        let (preview, preview_unavailable_reason) = if index < MAX_AGENT_TASK_OUTPUT_PREVIEW_FILES {
            output_preview(&context.op, &entry.path, &digest.sha256, digest.byte_size).await?
        } else {
            (None, Some("preview_limit".into()))
        };
        files.push(AgentTaskOutputReviewFile {
            task_path,
            byte_size: digest.byte_size,
            sha256: digest.sha256,
            preview,
            preview_unavailable_reason,
        });
    }

    Ok(AgentTaskOutputReviewOutput {
        workspace_id: context.workspace.id,
        workspace_name: context.workspace.name,
        task_id: request.task_id,
        task_root: context.task_root,
        file_count: files.len(),
        total_bytes,
        files,
    })
}

fn load_review_context(
    state: &AppState,
    request: &AgentTaskOutputReviewRequest,
) -> Result<ReviewContext, CoreError> {
    Uuid::parse_str(&request.task_id).map_err(|_| {
        CoreError::Config("Agent Task output review requires a valid task id".into())
    })?;
    Uuid::parse_str(&request.workspace_id).map_err(|_| {
        CoreError::Config("Agent Task output review requires a valid workspace id".into())
    })?;

    let workspace = state
        .workspaces
        .find_by_id(&request.workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }

    let storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(|_| {
            CoreError::Config("Agent Task workspace storage could not be validated".into())
        })?;
    if !storage.enabled {
        return Err(CoreError::Config(
            "Agent Task workspace storage must be enabled for output review".into(),
        ));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Task output review currently requires Local Filesystem workspace storage".into(),
        ));
    }

    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".into()))?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let op = state.operator_for_storage_id(&storage.id)?;
    let capabilities = op.info().capability();
    if !capabilities.stat || !capabilities.read || !capabilities.list {
        return Err(CoreError::Config(
            "Agent Task workspace lacks required read/list capabilities for output review".into(),
        ));
    }

    let task_root = agent_task_root(&request.task_id)?;
    let workspace_task_path = join_path(&workspace.root_path, &task_root);
    validate_local_path(&storage, &workspace.root_path)?;
    validate_local_path(&storage, &workspace_task_path)?;

    Ok(ReviewContext {
        workspace,
        storage,
        op,
        task_root,
        workspace_task_path,
    })
}

async fn validate_task_manifest(
    context: &ReviewContext,
    request: &AgentTaskOutputReviewRequest,
) -> Result<(), CoreError> {
    require_directory(
        &context.op,
        &context.workspace_task_path,
        "Agent Task package",
    )
    .await?;
    let manifest_path = join_path(&context.workspace_task_path, AGENT_TASK_MANIFEST_FILE);
    validate_local_path(&context.storage, &manifest_path)?;
    let metadata = context
        .op
        .stat(&manifest_path)
        .await
        .map_err(|_| CoreError::Config("Agent Task manifest could not be read".into()))?;
    if !metadata.is_file() {
        return Err(CoreError::Config(
            "Agent Task manifest is missing or is not a file".into(),
        ));
    }
    if metadata.content_length() > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }

    let manifest_bytes = context.op.read(&manifest_path).await?.to_vec();
    if manifest_bytes.len() as u64 > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest: AgentTaskManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_agent_task_manifest(&manifest)?;
    if manifest.task_id != request.task_id
        || manifest.workspace_id != request.workspace_id
        || manifest.task_root != context.task_root
    {
        return Err(CoreError::Config(
            "Agent Task manifest does not match the requested workspace task".into(),
        ));
    }
    Ok(())
}

async fn output_preview(
    op: &opendal::Operator,
    path: &str,
    expected_sha256: &str,
    byte_size: u64,
) -> Result<(Option<String>, Option<String>), CoreError> {
    if byte_size > MAX_AGENT_TASK_OUTPUT_PREVIEW_BYTES {
        return Ok((None, Some("too_large".into())));
    }

    let bytes = op
        .read(path)
        .await
        .map_err(|_| CoreError::Config("Agent Task output preview could not be read".into()))?
        .to_vec();
    if bytes.len() as u64 != byte_size {
        return Err(CoreError::Config(
            "Agent Task output changed while it was being reviewed".into(),
        ));
    }

    // Rehash after reading the preview so the displayed text and fingerprint are
    // from the same stable observation unless an external process races both reads.
    let after = hash_agent_task_file(op, path)
        .await
        .map_err(|_| CoreError::Config("Agent Task output bytes could not be reverified".into()))?;
    if after.byte_size != byte_size || after.sha256 != expected_sha256 {
        return Err(CoreError::Config(
            "Agent Task output changed while it was being reviewed".into(),
        ));
    }

    let Ok(text) = String::from_utf8(bytes) else {
        return Ok((None, Some("binary".into())));
    };
    if text
        .chars()
        .any(|ch| ch == '\0' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
    {
        return Ok((None, Some("binary".into())));
    }
    Ok((Some(text), None))
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

fn validate_local_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
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

fn relative_to_root(root: &str, path: &str) -> Result<String, CoreError> {
    let root = root.trim().trim_matches('/');
    let path = path.trim().trim_matches('/');
    if root.is_empty() {
        return (!path.is_empty())
            .then(|| path.to_string())
            .ok_or_else(|| CoreError::Config("Agent Task output path is invalid".into()));
    }
    path.strip_prefix(&format!("{root}/"))
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CoreError::Config("Agent Task output escaped the task package".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_output_path_must_stay_under_task_root() {
        assert_eq!(
            relative_to_root("agent/tasks/task", "agent/tasks/task/outputs/report.md").unwrap(),
            "outputs/report.md"
        );
        assert!(relative_to_root("agent/tasks/task", "agent/tasks/other/report.md").is_err());
    }

    #[test]
    fn task_output_review_types_do_not_encode_publication_state() {
        let file = AgentTaskOutputReviewFile {
            task_path: "outputs/report.md".into(),
            byte_size: 4,
            sha256: "a".repeat(64),
            preview: Some("test".into()),
            preview_unavailable_reason: None,
        };
        let json = serde_json::to_value(file).unwrap();
        assert!(json.get("destination").is_none());
        assert!(json.get("published").is_none());
        assert!(json.get("approved").is_none());
    }
}
