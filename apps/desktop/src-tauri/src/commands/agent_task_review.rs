use infimount_control::openai_hosted_request::{
    OPENAI_HOSTED_INLINE_MAX_BYTES, OPENAI_HOSTED_INLINE_MAX_FILES,
    OPENAI_HOSTED_INLINE_MAX_FILE_BYTES, OPENAI_HOSTED_PILOT_SESSION_SPEND_CENTS,
};
pub use infimount_control::review::{
    AgentTaskOutputReviewFile, AgentTaskOutputReviewOutput, AgentTaskOutputReviewRequest,
};
use infimount_control::review_evidence::{
    checked_reviewed_output_bytes, list_review_output_entries,
};
use infimount_control::sandbox_bundle::build_sandbox_task_bundle;
#[cfg(unix)]
use infimount_control::sandbox_local_export::{
    verify_prepared_hosted_input_scope, VerifiedHostedInputScope,
};
use infimount_core::agent_task_executions::{
    AdapterKind, AgentTaskExecutionRecord, ExecutionStatus,
};
use infimount_core::agent_task_io::hash_agent_task_file;
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskBrief, AgentTaskInput,
    AgentTaskManifest, AGENT_TASK_MANIFEST_FILE, AGENT_TASK_OUTPUTS_DIR,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_TASK_OUTPUT_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_AGENT_TASK_OUTPUT_PREVIEW_FILES: usize = 20;

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

    let file_entries = list_review_output_entries(&context.op, &outputs_path).await?;

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
        total_bytes = checked_reviewed_output_bytes(total_bytes, digest.byte_size)?;

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

/// Desktop-only, metadata-only review of a separately redacted export. It
/// neither reads prepared input bytes nor uploads them. A later trusted
/// transfer MUST independently rehash bytes and obtain separate consent.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewHostedTaskExportRequest {
    pub workspace_id: String,
    pub task_id: String,
    pub reviewed_export_brief: AgentTaskBrief,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedTaskExportPreview {
    pub provider: &'static str,
    pub data_residency: &'static str,
    pub zero_data_retention_supported: bool,
    pub sandbox_network_access: &'static str,
    pub pilot_spend_limit_usd_cents: u32,
    pub reviewed_title: String,
    pub reviewed_instructions: String,
    pub requested_outputs: Vec<String>,
    pub inputs: Vec<AgentTaskInput>,
    pub input_file_count: usize,
    pub input_total_bytes: u64,
    pub bundle_metadata_sha256: String,
    pub inputs_rehashed: bool,
    pub export_authorized: bool,
    pub executable: bool,
}

fn build_hosted_export_preview(
    manifest: &AgentTaskManifest,
    reviewed_export_brief: &AgentTaskBrief,
) -> Result<HostedTaskExportPreview, CoreError> {
    let bundle = build_sandbox_task_bundle(manifest, reviewed_export_brief)?;
    if bundle.requested_outputs.is_empty() {
        return Err(CoreError::Config(
            "Hosted export review requires an explicit output file list".into(),
        ));
    }
    if bundle.inputs.len() > OPENAI_HOSTED_INLINE_MAX_FILES
        || bundle.input_total_bytes > OPENAI_HOSTED_INLINE_MAX_BYTES as u64
        || bundle
            .inputs
            .iter()
            .any(|file| file.byte_size > OPENAI_HOSTED_INLINE_MAX_FILE_BYTES as u64)
    {
        return Err(CoreError::Config(
            "Prepared inputs exceed the first hosted inline pilot limits".into(),
        ));
    }

    Ok(HostedTaskExportPreview {
        provider: "openai_agents_api",
        data_residency: "United States",
        zero_data_retention_supported: false,
        sandbox_network_access: "disabled",
        pilot_spend_limit_usd_cents: OPENAI_HOSTED_PILOT_SESSION_SPEND_CENTS,
        reviewed_title: bundle.title,
        reviewed_instructions: bundle.instructions,
        requested_outputs: bundle.requested_outputs,
        input_file_count: bundle.inputs.len(),
        input_total_bytes: bundle.input_total_bytes,
        inputs: bundle.inputs,
        bundle_metadata_sha256: bundle.bundle_metadata_sha256,
        inputs_rehashed: false,
        export_authorized: false,
        executable: false,
    })
}

/// A read-only desktop preflight. Neither this command nor a returned digest
/// is a grant to transfer data, spend money, create sessions, or publish.
#[tauri::command]
pub async fn preview_hosted_task_export(
    state: State<'_, AppState>,
    request: PreviewHostedTaskExportRequest,
) -> Result<HostedTaskExportPreview, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config("Hosted export preview is unavailable while Infimount is degraded".into())
    })?;
    let local_request = AgentTaskOutputReviewRequest {
        workspace_id: request.workspace_id,
        task_id: request.task_id,
    };
    let context = load_review_context(&state, &local_request)?;
    let manifest = validate_task_manifest(&context, &local_request).await?;
    build_hosted_export_preview(&manifest, &request.reviewed_export_brief)
}

/// A read-only, sanitized view of PRIVATE hosted execution evidence.
/// Session IDs and provider credentials never cross the IPC boundary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedRecoveryStatus {
    RequiresFreshApproval,
    ManualSessionReconciliation,
    InspectPinnedSession,
    TerminalNeedsCleanupReview,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedExecutionHistoryItem {
    pub execution_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: ExecutionStatus,
    pub recovery: HostedRecoveryStatus,
    pub provider_session_recorded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedExecutionHistory {
    pub workspace_id: String,
    pub task_id: String,
    pub attempts: Vec<HostedExecutionHistoryItem>,
}

fn sanitized_hosted_attempt(record: AgentTaskExecutionRecord) -> HostedExecutionHistoryItem {
    let recovery = match record.status {
        ExecutionStatus::Prepared => HostedRecoveryStatus::RequiresFreshApproval,
        ExecutionStatus::Running if record.provider_execution_id.is_some() => {
            HostedRecoveryStatus::InspectPinnedSession
        }
        ExecutionStatus::Running => HostedRecoveryStatus::ManualSessionReconciliation,
        ExecutionStatus::Succeeded | ExecutionStatus::Failed | ExecutionStatus::Cancelled => {
            // The v1 private execution schema does not prove provider deletion.
            // Even a terminal attempt cannot prove the session was cleaned.
            HostedRecoveryStatus::TerminalNeedsCleanupReview
        }
    };
    HostedExecutionHistoryItem {
        execution_id: record.execution_id,
        started_at: record.started_at,
        finished_at: record.finished_at,
        status: record.status,
        recovery,
        provider_session_recorded: record.provider_execution_id.is_some(),
    }
}

/// Inspect the private evidence store after revalidating the selected task's
/// workspace and manifest. This does not contact a provider or authorize
/// another charged create. An unrecorded ID is NOT a reason to retry.
#[tauri::command]
pub async fn list_hosted_task_execution_status(
    state: State<'_, AppState>,
    request: AgentTaskOutputReviewRequest,
) -> Result<HostedExecutionHistory, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Hosted execution history is unavailable while Infimount is degraded".into(),
        )
    })?;
    let context = load_review_context(&state, &request)?;
    validate_task_manifest(&context, &request).await?;
    // ExecutionRecordStore uses a bounded cross-process file lock.
    // Keep blocking OS evidence reads off the Tauri async worker.
    let store = state.execution_records.clone();
    let workspace_id = request.workspace_id.clone();
    let task_id = request.task_id.clone();
    let records =
        tauri::async_runtime::spawn_blocking(move || store.list_for_task(&workspace_id, &task_id))
            .await
            .map_err(|_| {
                CoreError::Config("Private hosted execution history task failed".into())
            })??;
    let attempts = records
        .into_iter()
        .filter(|record| record.adapter_kind == AdapterKind::OpenaiSandbox)
        .map(sanitized_hosted_attempt)
        .collect();
    Ok(HostedExecutionHistory {
        workspace_id: request.workspace_id,
        task_id: request.task_id,
        attempts,
    })
}

/// Stronger read-only desktop verification than the C2.7 metadata preview.
/// The native code pins the current workspace configuration and reopens the
/// actual prepared bytes with O_NOFOLLOW. No file bytes, credentials or
/// authorization tokens are returned to the browser.
#[cfg(unix)]
#[tauri::command]
pub async fn verify_hosted_task_export(
    state: State<'_, AppState>,
    request: PreviewHostedTaskExportRequest,
) -> Result<VerifiedHostedInputScope, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config("Hosted input verification is unavailable during recovery".into())
    })?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _configuration = state
        .registry
        .acquire_configuration_transaction()
        .map_err(crate::state::mcp_error_to_core_error)?;
    state
        .recover_and_require_clean_configuration_locked()
        .map_err(crate::state::mcp_error_to_core_error)?;
    let _workspace = state.workspaces.acquire_mutation_lock()?;
    let local_request = AgentTaskOutputReviewRequest {
        workspace_id: request.workspace_id,
        task_id: request.task_id,
    };
    let context = load_review_context(&state, &local_request)?;
    let manifest = validate_task_manifest(&context, &local_request).await?;

    // A workspace root is storage-relative. Do NOT use that string directly
    // as a native path or accept an absolute path from the webview.
    infimount_mcp::storage_namespace::validate_local_mcp_path(
        &context.storage,
        &context.workspace.root_path,
    )
    .map_err(|_| CoreError::Config("Native workspace root is not confined".into()))?;
    let address = infimount_mcp::storage_namespace::storage_namespace_address(
        &context.storage,
        &context.workspace.root_path,
    )
    .map_err(|_| CoreError::Config("Native workspace identity could not be resolved".into()))?
    .ok_or_else(|| CoreError::Config("Native workspace root is not available".into()))?;
    let root = std::path::PathBuf::from(address.absolute_path);
    let reviewed_brief = request.reviewed_export_brief;

    // Reads are bounded but blocking. Keep the native lifecycle,
    // configuration and workspace locks held until the result is ready.
    tauri::async_runtime::spawn_blocking(move || {
        verify_prepared_hosted_input_scope(&root, &manifest, &reviewed_brief)
    })
    .await
    .map_err(|_| CoreError::Config("Native hosted input verification task failed".into()))?
}

/// Windows and other non-Unix platforms must be separately qualified with
/// reparse-safe descriptor semantics before this operation can be offered.
#[cfg(not(unix))]
#[tauri::command]
pub async fn verify_hosted_task_export(
    _state: State<'_, AppState>,
    _request: PreviewHostedTaskExportRequest,
) -> Result<serde_json::Value, CoreError> {
    Err(CoreError::Config(
        "Native prepared-byte verification is not qualified on this platform".into(),
    ))
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
) -> Result<AgentTaskManifest, CoreError> {
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
    Ok(manifest)
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
    fn hosted_export_preview_only_contains_separately_reviewed_metadata() {
        use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
        let manifest = AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            title: "PRIVATE original title".into(),
            created_at: "2026-11-08T12:00:00Z".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_root: "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            outputs_directory: "outputs".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/reviewed.csv".into(),
                byte_size: 5,
                sha256: "a".repeat(64),
            }],
        };
        let brief = AgentTaskBrief {
            title: "Approved synthetic description".into(),
            objective: "Summarize only the selected CSV.".into(),
            requested_outputs: vec!["outputs/summary.md".into()],
        };
        let preview = build_hosted_export_preview(&manifest, &brief).unwrap();
        assert_eq!(preview.input_file_count, 1);
        assert_eq!(preview.input_total_bytes, 5);
        assert_eq!(preview.pilot_spend_limit_usd_cents, 100);
        assert_eq!(preview.sandbox_network_access, "disabled");
        assert!(!preview.inputs_rehashed);
        assert!(!preview.export_authorized);
        assert!(!preview.executable);
        let wire = serde_json::to_string(&preview).unwrap();
        assert!(wire.contains("Approved synthetic description"));
        assert!(!wire.contains("PRIVATE original title"));
        assert!(!wire.contains(&manifest.workspace_id));
        assert!(!wire.contains("token"));
        let mut missing_outputs = brief.clone();
        missing_outputs.requested_outputs.clear();
        assert!(build_hosted_export_preview(&manifest, &missing_outputs).is_err());
        let mut over_limit = manifest.clone();
        over_limit.inputs[0].byte_size = (OPENAI_HOSTED_INLINE_MAX_FILE_BYTES + 1) as u64;
        assert!(build_hosted_export_preview(&over_limit, &brief).is_err());
    }

    #[test]
    fn hosted_history_projection_never_discloses_provider_session_id() {
        use infimount_core::agent_task_executions::{
            IsolationProfile, RuntimeKind, RuntimeProvider, EXECUTION_SCHEMA_VERSION,
        };
        let mut record = AgentTaskExecutionRecord {
            schema_version: EXECUTION_SCHEMA_VERSION,
            execution_id: "44444444-4444-4444-8444-444444444444".into(),
            task_id: "11111111-1111-4111-8111-111111111111".into(),
            workspace_id: "22222222-2222-4222-8222-222222222222".into(),
            adapter_kind: AdapterKind::OpenaiSandbox,
            adapter_version: "Agents API beta".into(),
            executable_sha256: None,
            runtime_kind: RuntimeKind::RemoteSandbox,
            runtime_provider: Some(RuntimeProvider::Openai),
            started_at: "2026-11-10T00:00:00Z".into(),
            finished_at: None,
            status: ExecutionStatus::Running,
            task_scope_digest: "a".repeat(64),
            enabled_task_tools: vec![],
            isolation_profile: IsolationProfile::RemoteBundleIsolated,
            prompt_sha256: "b".repeat(64),
            output_file_count: 0,
            output_total_bytes: 0,
            output_aggregate_sha256: None,
            error_code: None,
            provider_execution_id: Some("sess_private_never_in_ipc".into()),
        };
        record.validate().unwrap();
        let projection = sanitized_hosted_attempt(record.clone());
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(wire.contains("inspect_pinned_session"));
        assert!(projection.provider_session_recorded);
        assert!(!wire.contains("sess_private_never_in_ipc"));
        assert!(!wire.contains("providerExecutionId"));
        record.provider_execution_id = None;
        let unbound = serde_json::to_string(&sanitized_hosted_attempt(record)).unwrap();
        assert!(unbound.contains("manual_session_reconciliation"));
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
