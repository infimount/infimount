use std::collections::HashSet;
use std::path::{Path, PathBuf};

use infimount_core::agent_task_io::{copy_agent_task_file, hash_agent_task_file};
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASK_MANIFEST_FILE,
    AGENT_TASK_OUTPUTS_DIR, AGENT_TASK_PUBLISH_RECEIPT_FILE, AGENT_TASKS_DIR,
};
use infimount_core::workspaces::{workspace_schema_supported, WorkspaceRecord};
use infimount_core::{operations, CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_TASK_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DISCOVERED_OUTPUTS: usize = 10_000;
const MAX_PUBLISH_OUTPUTS: usize = 100;
const MAX_DESTINATION_PATH_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskSummary {
    pub task_id: String,
    pub title: String,
    pub created_at: String,
    pub workspace_id: String,
    pub task_root: String,
    pub prepared_files: usize,
    pub prepared_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputEntry {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskOutputList {
    pub task: AgentTaskSummary,
    pub outputs: Vec<AgentTaskOutputEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewedAgentTaskOutput {
    pub task_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishAgentTaskRequest {
    pub workspace_id: String,
    pub task_id: String,
    pub outputs: Vec<ReviewedAgentTaskOutput>,
    pub destination_storage_id: String,
    #[serde(default)]
    pub destination_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishAgentTaskItemResult {
    pub task_path: String,
    pub destination_path: String,
    pub sha256: String,
    pub byte_size: u64,
    pub status: String,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishAgentTaskResult {
    pub task_id: String,
    pub published: usize,
    pub conflicts: usize,
    pub stale: usize,
    pub failed: usize,
    pub receipt_written: bool,
    pub results: Vec<PublishAgentTaskItemResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishReceipt {
    schema_version: u32,
    task_id: String,
    published_at: String,
    destination_storage_id: String,
    results: Vec<PublishReceiptItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishReceiptItem {
    task_path: String,
    sha256: String,
    byte_size: u64,
    status: String,
}

struct WorkspaceTaskContext {
    workspace: WorkspaceRecord,
    storage: StorageRecord,
    op: Operator,
    manifest: AgentTaskManifest,
    task_root_absolute: String,
}

struct WorkspaceContext {
    workspace: WorkspaceRecord,
    storage: StorageRecord,
    op: Operator,
}

#[tauri::command]
pub async fn list_agent_tasks(
    state: State<'_, AppState>,
    workspaceId: String,
) -> Result<Vec<AgentTaskSummary>, CoreError> {
    state.require_operational().map_err(mcp_to_core)?;
    let context = load_workspace_context(&state, &workspaceId)?;
    let tasks_root = join_path(&context.workspace.root_path, AGENT_TASKS_DIR);
    validate_storage_path(&context.storage, &tasks_root)?;

    match context.op.stat(&tasks_root).await {
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_dir() => {
            return Err(CoreError::Config(
                "Agent Task tasks path exists as a file".to_string(),
            ))
        }
        Ok(_) => {}
    }

    let entries = operations::list_entries(&context.op, &tasks_root).await?;
    let mut tasks = Vec::new();
    for entry in entries.into_iter().filter(|entry| entry.is_dir) {
        let Some(task_id) = entry
            .path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string)
        else {
            continue;
        };
        if Uuid::parse_str(&task_id).is_err() {
            continue;
        }
        let task_root = match agent_task_root(&task_id) {
            Ok(root) => root,
            Err(_) => continue,
        };
        let task_absolute = join_path(&context.workspace.root_path, &task_root);
        let manifest_path = join_path(&task_absolute, AGENT_TASK_MANIFEST_FILE);
        let manifest = match read_manifest(&context.op, &manifest_path).await {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if manifest.task_id != task_id
            || manifest.workspace_id != context.workspace.id
            || manifest.task_root != task_root
        {
            continue;
        }
        tasks.push(summary_from_manifest(&manifest));
    }

    tasks.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(tasks)
}

#[tauri::command]
pub async fn list_agent_task_outputs(
    state: State<'_, AppState>,
    workspaceId: String,
    taskId: String,
) -> Result<AgentTaskOutputList, CoreError> {
    state.require_operational().map_err(mcp_to_core)?;
    let context = load_task_context(&state, &workspaceId, &taskId).await?;
    let outputs_root = join_path(&context.task_root_absolute, AGENT_TASK_OUTPUTS_DIR);
    validate_storage_path(&context.storage, &outputs_root)?;

    let entries = match context.op.stat(&outputs_root).await {
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_dir() => {
            return Err(CoreError::Config(
                "Agent Task outputs path exists as a file".to_string(),
            ))
        }
        Ok(_) => operations::list_entries_recursive(&context.op, &outputs_root).await?,
    };
    if entries.len() >= MAX_DISCOVERED_OUTPUTS {
        return Err(CoreError::Config(
            "Agent Task outputs exceed the review discovery limit".to_string(),
        ));
    }

    let mut outputs = Vec::new();
    for entry in entries.into_iter().filter(|entry| !entry.is_dir) {
        validate_storage_path(&context.storage, &entry.path)?;
        let task_path = relative_to_root(&context.task_root_absolute, &entry.path)?;
        validate_output_path(&task_path)?;
        let digest = hash_agent_task_file(&context.op, &entry.path).await?;
        outputs.push(AgentTaskOutputEntry {
            task_path,
            byte_size: digest.byte_size,
            sha256: digest.sha256,
            modified_at: entry.modified_at,
        });
    }
    outputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));

    record_task_event(
        &state,
        infimount_mcp::telemetry::ProductEventName::AgentTaskReviewOpened,
        true,
        None,
    );
    Ok(AgentTaskOutputList {
        task: summary_from_manifest(&context.manifest),
        outputs,
    })
}

#[tauri::command]
pub async fn publish_agent_task_outputs(
    state: State<'_, AppState>,
    request: PublishAgentTaskRequest,
) -> Result<PublishAgentTaskResult, CoreError> {
    state.require_operational().map_err(mcp_to_core)?;
    validate_publish_request(&request)?;
    let context = load_task_context(&state, &request.workspace_id, &request.task_id).await?;
    let destination_storage = state
        .find_storage_by_id(&request.destination_storage_id)
        .map_err(mcp_to_core)?;
    if !destination_storage.enabled {
        return Err(CoreError::Config(
            "Agent Task publication destination is disabled".to_string(),
        ));
    }
    if destination_storage.read_only {
        return Err(CoreError::Config(
            "Agent Task publication destination is read-only".to_string(),
        ));
    }
    let destination_op = state.operator_for_storage_id(&destination_storage.id)?;
    let destination_caps = destination_op.info().capability();
    if !destination_caps.stat || !destination_caps.write || !destination_caps.write_with_if_not_exists {
        return Err(CoreError::Config(
            "Agent Task publication requires destination support for atomic no-overwrite writes"
                .to_string(),
        ));
    }

    let destination_dir = normalize_destination_dir(&request.destination_dir)?;
    validate_storage_path(&destination_storage, &destination_dir)?;
    let (staging_root, staging_op) = secure_publish_staging(&state)?;

    record_task_event(
        &state,
        infimount_mcp::telemetry::ProductEventName::AgentTaskPublishStarted,
        true,
        None,
    );

    let mut results = Vec::with_capacity(request.outputs.len());
    for reviewed in &request.outputs {
        let source_path = join_path(&context.task_root_absolute, &reviewed.task_path);
        validate_storage_path(&context.storage, &source_path)?;
        let relative_output = reviewed
            .task_path
            .strip_prefix(&format!("{AGENT_TASK_OUTPUTS_DIR}/"))
            .ok_or_else(|| CoreError::Config("invalid Agent Task output path".to_string()))?;
        let destination_path = join_path(&destination_dir, relative_output);
        validate_storage_path(&destination_storage, &destination_path)?;
        reject_publish_namespace_overlap(
            &context.storage,
            &source_path,
            &destination_storage,
            &destination_path,
        )?;

        let snapshot_name = format!("{}.snapshot", Uuid::new_v4());
        let snapshot_path = snapshot_name.as_str();
        let snapshot = copy_agent_task_file(
            &context.op,
            &source_path,
            &staging_op,
            snapshot_path,
            true,
        )
        .await;

        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                results.push(failed_item(
                    reviewed,
                    destination_path,
                    "failed",
                    core_error_code(&error),
                    0,
                ));
                continue;
            }
        };

        if snapshot.sha256 != reviewed.sha256 {
            let _ = operations::delete(&staging_op, snapshot_path).await;
            results.push(PublishAgentTaskItemResult {
                task_path: reviewed.task_path.clone(),
                destination_path,
                sha256: snapshot.sha256,
                byte_size: snapshot.byte_size,
                status: "stale".to_string(),
                error_code: Some("OUTPUT_CHANGED".to_string()),
            });
            continue;
        }

        let parent_result = ensure_parent_directories(&destination_op, &destination_storage, &destination_path).await;
        if let Err(error) = parent_result {
            let _ = operations::delete(&staging_op, snapshot_path).await;
            results.push(failed_item(
                reviewed,
                destination_path,
                "failed",
                core_error_code(&error),
                snapshot.byte_size,
            ));
            continue;
        }

        let publish_result = copy_agent_task_file(
            &staging_op,
            snapshot_path,
            &destination_op,
            &destination_path,
            true,
        )
        .await;
        let _ = operations::delete(&staging_op, snapshot_path).await;

        match publish_result {
            Ok(published) if published.sha256 == reviewed.sha256 => {
                results.push(PublishAgentTaskItemResult {
                    task_path: reviewed.task_path.clone(),
                    destination_path,
                    sha256: reviewed.sha256.clone(),
                    byte_size: published.byte_size,
                    status: "published".to_string(),
                    error_code: None,
                });
            }
            Ok(published) => {
                results.push(PublishAgentTaskItemResult {
                    task_path: reviewed.task_path.clone(),
                    destination_path,
                    sha256: published.sha256,
                    byte_size: published.byte_size,
                    status: "failed".to_string(),
                    error_code: Some("PUBLISHED_DIGEST_MISMATCH".to_string()),
                });
            }
            Err(error) if is_already_exists(&error) => {
                results.push(failed_item(
                    reviewed,
                    destination_path,
                    "conflict",
                    Some("ALREADY_EXISTS".to_string()),
                    snapshot.byte_size,
                ));
            }
            Err(error) => {
                results.push(failed_item(
                    reviewed,
                    destination_path,
                    "failed",
                    core_error_code(&error),
                    snapshot.byte_size,
                ));
            }
        }
    }

    // Best-effort cleanup of any abandoned staging entries created by this run.
    let _ = std::fs::remove_dir(&staging_root);

    let published = results.iter().filter(|item| item.status == "published").count();
    let conflicts = results.iter().filter(|item| item.status == "conflict").count();
    let stale = results.iter().filter(|item| item.status == "stale").count();
    let failed = results.iter().filter(|item| item.status == "failed").count();

    let receipt = PublishReceipt {
        schema_version: 1,
        task_id: request.task_id.clone(),
        published_at: chrono::Utc::now().to_rfc3339(),
        destination_storage_id: request.destination_storage_id.clone(),
        results: results
            .iter()
            .map(|item| PublishReceiptItem {
                task_path: item.task_path.clone(),
                sha256: item.sha256.clone(),
                byte_size: item.byte_size,
                status: item.status.clone(),
            })
            .collect(),
    };
    let receipt_written = write_receipt(&context, &receipt).await.is_ok();

    record_task_event(
        &state,
        infimount_mcp::telemetry::ProductEventName::AgentTaskPublishCompleted,
        failed == 0 && stale == 0 && conflicts == 0,
        (failed > 0 || stale > 0 || conflicts > 0).then_some("publish"),
    );
    if published > 0 && failed == 0 && stale == 0 && conflicts == 0 {
        record_task_event(
            &state,
            infimount_mcp::telemetry::ProductEventName::AgentTaskCompleted,
            true,
            None,
        );
    }

    Ok(PublishAgentTaskResult {
        task_id: request.task_id,
        published,
        conflicts,
        stale,
        failed,
        receipt_written,
        results,
    })
}

fn validate_publish_request(request: &PublishAgentTaskRequest) -> Result<(), CoreError> {
    Uuid::parse_str(&request.workspace_id)
        .map_err(|_| CoreError::Config("Agent Task workspace id is invalid".to_string()))?;
    Uuid::parse_str(&request.task_id)
        .map_err(|_| CoreError::Config("Agent Task id is invalid".to_string()))?;
    if request.destination_storage_id.trim().is_empty() {
        return Err(CoreError::Config(
            "Agent Task publication destination storage is required".to_string(),
        ));
    }
    if request.outputs.is_empty() || request.outputs.len() > MAX_PUBLISH_OUTPUTS {
        return Err(CoreError::Config(format!(
            "Agent Task publication must select between 1 and {MAX_PUBLISH_OUTPUTS} outputs"
        )));
    }
    let mut seen = HashSet::with_capacity(request.outputs.len());
    for output in &request.outputs {
        validate_output_path(&output.task_path)?;
        validate_sha256(&output.sha256)?;
        if !seen.insert(output.task_path.to_lowercase()) {
            return Err(CoreError::Config(
                "Agent Task publication contains duplicate output paths".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_output_path(path: &str) -> Result<(), CoreError> {
    if path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(|ch| ch == '\0' || ch.is_control())
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !path.starts_with(&format!("{AGENT_TASK_OUTPUTS_DIR}/"))
    {
        return Err(CoreError::Config(
            "Agent Task output path must be a canonical child of outputs/".to_string(),
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), CoreError> {
    if value.len() != 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, 'a'..='f'))
    {
        return Err(CoreError::Config(
            "Agent Task reviewed SHA-256 is invalid".to_string(),
        ));
    }
    Ok(())
}

fn normalize_destination_dir(path: &str) -> Result<String, CoreError> {
    let path = path.trim().trim_matches('/');
    if path.len() > MAX_DESTINATION_PATH_BYTES
        || path.contains('\\')
        || path.chars().any(|ch| ch == '\0' || ch.is_control())
        || (!path.is_empty()
            && path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == ".."))
    {
        return Err(CoreError::Config(
            "Agent Task destination directory is invalid".to_string(),
        ));
    }
    Ok(path.to_string())
}

fn load_workspace_context(state: &AppState, workspace_id: &str) -> Result<WorkspaceContext, CoreError> {
    let workspace = state
        .workspaces
        .find_by_id(workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".to_string()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".to_string(),
        ));
    }
    let storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(mcp_to_core)?;
    if !storage.enabled {
        return Err(CoreError::Config(
            "Agent Task workspace storage is disabled".to_string(),
        ));
    }
    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".to_string()))?;
    if !matches!(kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Tasks currently require a Local Filesystem Agent Workspace".to_string(),
        ));
    }
    let namespace = infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
        .map_err(|_| CoreError::Config("failed to verify Agent Task workspace identity".to_string()))?;
    if namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".to_string(),
        ));
    }
    validate_storage_path(&storage, &workspace.root_path)?;
    let op = state.operator_for_storage_id(&storage.id)?;
    Ok(WorkspaceContext { workspace, storage, op })
}

async fn load_task_context(
    state: &AppState,
    workspace_id: &str,
    task_id: &str,
) -> Result<WorkspaceTaskContext, CoreError> {
    Uuid::parse_str(task_id)
        .map_err(|_| CoreError::Config("Agent Task id is invalid".to_string()))?;
    let workspace_context = load_workspace_context(state, workspace_id)?;
    let task_root = agent_task_root(task_id)?;
    let task_root_absolute = join_path(&workspace_context.workspace.root_path, &task_root);
    validate_storage_path(&workspace_context.storage, &task_root_absolute)?;
    let manifest_path = join_path(&task_root_absolute, AGENT_TASK_MANIFEST_FILE);
    let manifest = read_manifest(&workspace_context.op, &manifest_path).await?;
    if manifest.task_id != task_id
        || manifest.workspace_id != workspace_context.workspace.id
        || manifest.task_root != task_root
    {
        return Err(CoreError::Config(
            "Agent Task manifest does not match its containing workspace/task".to_string(),
        ));
    }
    Ok(WorkspaceTaskContext {
        workspace: workspace_context.workspace,
        storage: workspace_context.storage,
        op: workspace_context.op,
        manifest,
        task_root_absolute,
    })
}

async fn read_manifest(op: &Operator, path: &str) -> Result<AgentTaskManifest, CoreError> {
    let before = op.stat(path).await?;
    if before.is_dir() || before.content_length() > MAX_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is missing or exceeds its bounded size".to_string(),
        ));
    }
    let bytes = op.read(path).await?.to_vec();
    let after = op.stat(path).await?;
    if after.is_dir()
        || after.content_length() != before.content_length()
        || bytes.len() as u64 != after.content_length()
    {
        return Err(CoreError::Config(
            "Agent Task manifest changed while being read".to_string(),
        ));
    }
    let manifest: AgentTaskManifest = serde_json::from_slice(&bytes)?;
    validate_agent_task_manifest(&manifest)?;
    Ok(manifest)
}

fn summary_from_manifest(manifest: &AgentTaskManifest) -> AgentTaskSummary {
    AgentTaskSummary {
        task_id: manifest.task_id.clone(),
        title: manifest.title.clone(),
        created_at: manifest.created_at.clone(),
        workspace_id: manifest.workspace_id.clone(),
        task_root: manifest.task_root.clone(),
        prepared_files: manifest.inputs.len(),
        prepared_bytes: manifest
            .inputs
            .iter()
            .fold(0_u64, |total, input| total.saturating_add(input.byte_size)),
    }
}

async fn ensure_parent_directories(
    op: &Operator,
    storage: &StorageRecord,
    destination_path: &str,
) -> Result<(), CoreError> {
    let Some((parent, _)) = destination_path.rsplit_once('/') else {
        return Ok(());
    };
    if parent.is_empty() {
        return Ok(());
    }
    let mut current = String::new();
    for segment in parent.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        validate_storage_path(storage, &current)?;
        match op.stat(&current).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(CoreError::Config(
                    "Agent Task publication parent exists as a file".to_string(),
                ))
            }
            Err(error) if error.kind() == opendal::ErrorKind::NotFound => {
                operations::create_directory(op, &current).await?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn reject_publish_namespace_overlap(
    source_storage: &StorageRecord,
    source_path: &str,
    destination_storage: &StorageRecord,
    destination_path: &str,
) -> Result<(), CoreError> {
    let relation = infimount_mcp::storage_namespace::transfer_namespace_relation(
        source_storage,
        source_path,
        destination_storage,
        destination_path,
    )
    .map_err(|_| CoreError::Config("failed to compare publication namespaces".to_string()))?;
    if infimount_mcp::storage_namespace::transfer_has_namespace_conflict(&relation) {
        return Err(CoreError::Config(
            "Agent Task publication destination overlaps the reviewed output".to_string(),
        ));
    }
    Ok(())
}

fn secure_publish_staging(state: &AppState) -> Result<(PathBuf, Operator), CoreError> {
    let root = state.workspaces.dir().join("agent-task-publish-staging");
    std::fs::create_dir_all(&root)?;
    let metadata = std::fs::symlink_metadata(&root)?;
    if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(CoreError::Config(
            "Agent Task publication staging path is unsafe".to_string(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(CoreError::Config(
                "Agent Task publication staging path is not owned by the current user".to_string(),
            ));
        }
        if metadata.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let root_text = root.to_string_lossy().to_string();
    let op = Operator::new(opendal::services::Fs::default().root(&root_text))?;
    Ok((root, op))
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

async fn write_receipt(context: &WorkspaceTaskContext, receipt: &PublishReceipt) -> Result<(), CoreError> {
    let path = join_path(&context.task_root_absolute, AGENT_TASK_PUBLISH_RECEIPT_FILE);
    validate_storage_path(&context.storage, &path)?;
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    operations::write_full(&context.op, &path, &bytes).await
}

fn failed_item(
    reviewed: &ReviewedAgentTaskOutput,
    destination_path: String,
    status: &str,
    error_code: Option<String>,
    byte_size: u64,
) -> PublishAgentTaskItemResult {
    PublishAgentTaskItemResult {
        task_path: reviewed.task_path.clone(),
        destination_path,
        sha256: reviewed.sha256.clone(),
        byte_size,
        status: status.to_string(),
        error_code,
    }
}

fn is_already_exists(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Storage(inner)
            if matches!(
                inner.kind(),
                opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch
            )
    ) || matches!(error, CoreError::Io(inner) if inner.kind() == std::io::ErrorKind::AlreadyExists)
}

fn core_error_code(error: &CoreError) -> Option<String> {
    Some(match error.code() {
        infimount_core::models::ErrorCode::NotFound => "NOT_FOUND",
        infimount_core::models::ErrorCode::PermissionDenied => "PERMISSION_DENIED",
        infimount_core::models::ErrorCode::AlreadyExists => "ALREADY_EXISTS",
        infimount_core::models::ErrorCode::ConfigError => "CONFIG_ERROR",
        infimount_core::models::ErrorCode::IoError => "IO_ERROR",
        infimount_core::models::ErrorCode::Unknown => "UNKNOWN",
    }
    .to_string())
}

fn validate_storage_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::storage_namespace_address(storage, path)
        .map_err(|_| CoreError::Config("Agent Task storage path is invalid".to_string()))?
        .ok_or_else(|| CoreError::Config("Agent Task storage path could not be resolved".to_string()))?;
    if matches!(storage.backend.trim().to_ascii_lowercase().as_str(), "local" | "fs") {
        infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path).map_err(|_| {
            CoreError::Config("Agent Task local path confinement check failed".to_string())
        })?;
    }
    Ok(())
}

fn relative_to_root(root: &str, path: &str) -> Result<String, CoreError> {
    let root = root.trim_matches('/');
    let path = path.trim_matches('/');
    let prefix = format!("{root}/");
    path.strip_prefix(&prefix)
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CoreError::Config("Agent Task path escaped its package root".to_string()))
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

fn record_task_event(
    state: &AppState,
    name: infimount_mcp::telemetry::ProductEventName,
    success: bool,
    failure_stage: Option<&str>,
) {
    let mut event = infimount_mcp::telemetry::ProductEvent::new(name);
    event.success = Some(success);
    event.failure_stage = failure_stage.map(str::to_string);
    let _ = state.product_events.record(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_are_canonical_children() {
        assert!(validate_output_path("outputs/report.md").is_ok());
        assert!(validate_output_path("outputs/data/report.csv").is_ok());
        assert!(validate_output_path("report.md").is_err());
        assert!(validate_output_path("outputs/../secret").is_err());
        assert!(validate_output_path("outputs\\report.md").is_err());
    }

    #[test]
    fn destination_directory_rejects_escape() {
        assert_eq!(normalize_destination_dir("/reports/2026/").unwrap(), "reports/2026");
        assert!(normalize_destination_dir("reports/../private").is_err());
        assert!(normalize_destination_dir("reports\\private").is_err());
    }

    #[test]
    fn publish_selection_rejects_case_collisions() {
        let request = PublishAgentTaskRequest {
            workspace_id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            outputs: vec![
                ReviewedAgentTaskOutput {
                    task_path: "outputs/a.md".to_string(),
                    sha256: "a".repeat(64),
                },
                ReviewedAgentTaskOutput {
                    task_path: "outputs/A.md".to_string(),
                    sha256: "b".repeat(64),
                },
            ],
            destination_storage_id: "storage".to_string(),
            destination_dir: String::new(),
        };
        assert!(validate_publish_request(&request).is_err());
    }
}
