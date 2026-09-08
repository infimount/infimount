use std::collections::HashSet;

use infimount_core::agent_task_io::{
    hash_agent_task_file, publish_reviewed_agent_task_file, AgentTaskFileDigest,
};
use infimount_core::agent_tasks::{
    agent_task_root, validate_agent_task_manifest, AgentTaskManifest, AGENT_TASKS_DIR,
    AGENT_TASK_MANIFEST_FILE,
};
use infimount_core::workspaces::workspace_schema_supported;
use infimount_core::{CoreError, SourceKind};
use infimount_mcp::registry::StorageRecord;
use opendal::ErrorKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

const AGENT_TASK_PUBLICATION_SCHEMA_VERSION: u32 = 1;
const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_TASK_PUBLISH_FILES: usize = 100;
const MAX_AGENT_TASK_PUBLISH_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_AGENT_TASK_DESTINATION_PATH_BYTES: usize = 4 * 1024;
const AGENT_TASK_PUBLISH_RECEIPT_PREFIX: &str = "publish-receipt-";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewedAgentTaskOutput {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskPublishConflictPolicy {
    Fail,
    Rename,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTaskPublicationRequest {
    pub workspace_id: String,
    pub task_id: String,
    pub outputs: Vec<ReviewedAgentTaskOutput>,
    pub destination_storage_id: String,
    #[serde(default)]
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyAgentTaskPublicationRequest {
    pub publication: AgentTaskPublicationRequest,
    pub preview_token: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskPublishAction {
    Create,
    Rename,
    Conflict,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationPlanFile {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub destination_path: String,
    pub action: AgentTaskPublishAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationPreview {
    pub workspace_id: String,
    pub workspace_name: String,
    pub task_id: String,
    pub task_root: String,
    pub destination_storage_id: String,
    pub destination_storage_name: String,
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
    pub file_count: usize,
    pub total_bytes: u64,
    pub create_count: usize,
    pub rename_count: usize,
    pub conflict_count: usize,
    pub can_publish: bool,
    pub preview_token: String,
    pub files: Vec<AgentTaskPublicationPlanFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationOutput {
    pub publication_id: String,
    pub published_at: String,
    pub workspace_id: String,
    pub task_id: String,
    pub destination_storage_id: String,
    pub destination_storage_name: String,
    pub receipt_path: String,
    pub files: Vec<AgentTaskPublicationPlanFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTaskPublishReceipt {
    schema_version: u32,
    publication_id: String,
    task_id: String,
    workspace_id: String,
    published_at: String,
    destination_storage_id: String,
    destination_storage_name: String,
    destination_dir: String,
    conflict_policy: AgentTaskPublishConflictPolicy,
    files: Vec<AgentTaskPublicationPlanFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentTaskPublicationFingerprint<'a> {
    schema_version: u32,
    workspace_id: &'a str,
    task_id: &'a str,
    workspace_namespace: &'a str,
    destination_storage_id: &'a str,
    destination_namespace: &'a str,
    destination_dir: &'a str,
    conflict_policy: &'a AgentTaskPublishConflictPolicy,
    files: &'a [AgentTaskPublicationPlanFile],
}

struct PublicationContext {
    workspace: infimount_core::workspaces::WorkspaceRecord,
    workspace_storage: StorageRecord,
    destination_storage: StorageRecord,
    workspace_op: opendal::Operator,
    destination_op: opendal::Operator,
    workspace_namespace: String,
    destination_namespace: String,
    task_root: String,
    workspace_task_path: String,
    destination_dir: String,
}

struct PublicationPlan {
    context: PublicationContext,
    files: Vec<AgentTaskPublicationPlanFile>,
    preview_token: String,
    total_bytes: u64,
    create_count: usize,
    rename_count: usize,
    conflict_count: usize,
}

#[tauri::command]
pub async fn preview_agent_task_publication(
    state: State<'_, AppState>,
    request: AgentTaskPublicationRequest,
) -> Result<AgentTaskPublicationPreview, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Agent Task publication is unavailable while Infimount is degraded".into(),
        )
    })?;
    let plan = build_publication_plan(&state, &request).await?;
    let context = &plan.context;

    Ok(AgentTaskPublicationPreview {
        workspace_id: context.workspace.id.clone(),
        workspace_name: context.workspace.name.clone(),
        task_id: request.task_id,
        task_root: context.task_root.clone(),
        destination_storage_id: context.destination_storage.id.clone(),
        destination_storage_name: context.destination_storage.name.clone(),
        destination_dir: context.destination_dir.clone(),
        conflict_policy: request.conflict_policy,
        file_count: plan.files.len(),
        total_bytes: plan.total_bytes,
        create_count: plan.create_count,
        rename_count: plan.rename_count,
        conflict_count: plan.conflict_count,
        can_publish: plan.conflict_count == 0,
        preview_token: plan.preview_token,
        files: plan.files,
    })
}

#[tauri::command]
pub async fn publish_agent_task_outputs(
    state: State<'_, AppState>,
    request: ApplyAgentTaskPublicationRequest,
) -> Result<AgentTaskPublicationOutput, CoreError> {
    state.require_operational().map_err(|_| {
        CoreError::Config(
            "Agent Task publication is unavailable while Infimount is degraded".into(),
        )
    })?;
    validate_preview_token(&request.preview_token)?;

    // Freeze storage configuration for the complete apply transaction. This
    // follows the repository's existing mutation lock order and prevents either
    // the task workspace or destination storage from being repointed after the
    // approved plan is rebuilt but before its bytes and receipt are committed.
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let _config_transaction = state
        .registry
        .acquire_configuration_transaction()
        .map_err(crate::state::mcp_error_to_core_error)?;
    state
        .recover_and_require_clean_configuration_locked()
        .map_err(crate::state::mcp_error_to_core_error)?;

    // Serialize workspace create/update/delete against the complete publication
    // transaction. File browsing and ordinary storage reads remain unaffected.
    let _workspace_transaction = state.workspaces.acquire_mutation_lock()?;

    // Rebuild the complete plan from current source bytes and current destination
    // state. The preview token is a staleness token, not an authorization token.
    let plan = build_publication_plan(&state, &request.publication).await?;
    if plan.preview_token != request.preview_token {
        return Err(CoreError::Config(
            "Agent Task publication changed since review; review the publication plan again".into(),
        ));
    }
    if plan.conflict_count != 0 {
        return Err(CoreError::Config(
            "Agent Task publication has unresolved destination conflicts".into(),
        ));
    }

    let mut published = Vec::<AgentTaskPublicationPlanFile>::new();
    for file in &plan.files {
        let source_path = join_path(&plan.context.workspace_task_path, &file.task_path);
        validate_local_path(&plan.context.workspace_storage, &source_path)?;
        validate_destination_path(&plan.context.destination_storage, &file.destination_path)?;
        ensure_publication_parent(
            &plan.context.destination_op,
            &plan.context.destination_storage,
            &file.destination_path,
        )
        .await?;

        let expected = AgentTaskFileDigest {
            byte_size: file.byte_size,
            sha256: file.sha256.clone(),
        };
        if let Err(error) = publish_reviewed_agent_task_file(
            &plan.context.workspace_op,
            &source_path,
            &plan.context.destination_op,
            &file.destination_path,
            &expected,
        )
        .await
        {
            if published.is_empty() {
                return Err(error);
            }
            // Some create-only destinations are already committed. OpenDAL 0.58
            // has no portable compare-and-delete primitive, so automatic rollback
            // could delete data replaced by another writer after our verification.
            return Err(CoreError::TransferCleanupRequired);
        }
        published.push(file.clone());
    }

    let publication_id = Uuid::new_v4().to_string();
    let published_at = chrono::Utc::now().to_rfc3339();
    let receipt = AgentTaskPublishReceipt {
        schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
        publication_id: publication_id.clone(),
        task_id: request.publication.task_id.clone(),
        workspace_id: request.publication.workspace_id.clone(),
        published_at: published_at.clone(),
        destination_storage_id: plan.context.destination_storage.id.clone(),
        destination_storage_name: plan.context.destination_storage.name.clone(),
        destination_dir: plan.context.destination_dir.clone(),
        conflict_policy: request.publication.conflict_policy,
        files: published.clone(),
    };
    let receipt_file = format!("{AGENT_TASK_PUBLISH_RECEIPT_PREFIX}{publication_id}.json");
    let receipt_path = join_path(&plan.context.workspace_task_path, &receipt_file);
    validate_local_path(&plan.context.workspace_storage, &receipt_path)?;
    let mut receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    receipt_bytes.push(b'\n');
    if write_publication_receipt(&plan.context.workspace_op, &receipt_path, &receipt_bytes)
        .await
        .is_err()
    {
        // Outputs have already committed. Preserve them and any ambiguous
        // receipt state for explicit cleanup rather than deleting by path.
        return Err(CoreError::TransferCleanupRequired);
    }

    Ok(AgentTaskPublicationOutput {
        publication_id,
        published_at,
        workspace_id: request.publication.workspace_id,
        task_id: request.publication.task_id,
        destination_storage_id: plan.context.destination_storage.id,
        destination_storage_name: plan.context.destination_storage.name,
        receipt_path: relative_to_root(&plan.context.workspace.root_path, &receipt_path)?,
        files: published,
    })
}

async fn build_publication_plan(
    state: &AppState,
    request: &AgentTaskPublicationRequest,
) -> Result<PublicationPlan, CoreError> {
    let outputs = validate_publication_request(request)?;
    let context = load_publication_context(state, request)?;
    validate_task_manifest(&context, request).await?;

    let mut files = Vec::with_capacity(outputs.len());
    let mut total_bytes = 0u64;
    let mut create_count = 0usize;
    let mut rename_count = 0usize;
    let mut conflict_count = 0usize;
    let mut destination_keys = HashSet::with_capacity(outputs.len());

    for reviewed in outputs {
        let source_path = join_path(&context.workspace_task_path, &reviewed.task_path);
        validate_local_path(&context.workspace_storage, &source_path)?;
        let metadata = context.workspace_op.stat(&source_path).await.map_err(|_| {
            CoreError::Config("A reviewed Agent Task output is no longer available".into())
        })?;
        if !metadata.is_file() || metadata.content_length() != reviewed.byte_size {
            return Err(CoreError::Config(
                "Agent Task output changed since it was reviewed".into(),
            ));
        }
        let current = hash_agent_task_file(&context.workspace_op, &source_path).await?;
        if current.byte_size != reviewed.byte_size || current.sha256 != reviewed.sha256 {
            return Err(CoreError::Config(
                "Agent Task output changed since it was reviewed".into(),
            ));
        }
        total_bytes = total_bytes.checked_add(current.byte_size).ok_or_else(|| {
            CoreError::Config("Agent Task publication byte count overflowed".into())
        })?;
        if total_bytes > MAX_AGENT_TASK_PUBLISH_BYTES {
            return Err(CoreError::Config(
                "Agent Task publication exceeds the 2 GiB limit".into(),
            ));
        }

        let relative_output = reviewed
            .task_path
            .strip_prefix("outputs/")
            .ok_or_else(|| CoreError::Config("Agent Task publication path is invalid".into()))?;
        let base_destination = join_path(&context.destination_dir, relative_output);
        validate_destination_path(&context.destination_storage, &base_destination)?;
        reject_task_destination_overlap(&context, &source_path, &base_destination)?;

        let (destination_path, action) =
            match path_exists(&context.destination_op, &base_destination).await? {
                false => (base_destination, AgentTaskPublishAction::Create),
                true if request.conflict_policy == AgentTaskPublishConflictPolicy::Fail => {
                    (base_destination, AgentTaskPublishAction::Conflict)
                }
                true => (
                    unique_destination_path(&context.destination_op, &base_destination).await?,
                    AgentTaskPublishAction::Rename,
                ),
            };
        validate_destination_path(&context.destination_storage, &destination_path)?;
        reject_task_destination_overlap(&context, &source_path, &destination_path)?;
        let destination_key = destination_path.to_lowercase();
        if !destination_keys.insert(destination_key) {
            return Err(CoreError::Config(
                "Selected Agent Task outputs collide at the publication destination".into(),
            ));
        }

        match action {
            AgentTaskPublishAction::Create => create_count += 1,
            AgentTaskPublishAction::Rename => rename_count += 1,
            AgentTaskPublishAction::Conflict => conflict_count += 1,
        }
        files.push(AgentTaskPublicationPlanFile {
            task_path: reviewed.task_path,
            byte_size: reviewed.byte_size,
            sha256: reviewed.sha256,
            destination_path,
            action,
        });
    }
    files.sort_by(|left, right| left.task_path.cmp(&right.task_path));

    let fingerprint = AgentTaskPublicationFingerprint {
        schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
        workspace_id: &request.workspace_id,
        task_id: &request.task_id,
        workspace_namespace: &context.workspace_namespace,
        destination_storage_id: &context.destination_storage.id,
        destination_namespace: &context.destination_namespace,
        destination_dir: &context.destination_dir,
        conflict_policy: &request.conflict_policy,
        files: &files,
    };
    let preview_token = sha256_json(&fingerprint)?;

    Ok(PublicationPlan {
        context,
        files,
        preview_token,
        total_bytes,
        create_count,
        rename_count,
        conflict_count,
    })
}

fn validate_publication_request(
    request: &AgentTaskPublicationRequest,
) -> Result<Vec<ReviewedAgentTaskOutput>, CoreError> {
    Uuid::parse_str(&request.workspace_id).map_err(|_| {
        CoreError::Config("Agent Task publication requires a valid workspace id".into())
    })?;
    Uuid::parse_str(&request.task_id)
        .map_err(|_| CoreError::Config("Agent Task publication requires a valid task id".into()))?;
    if request.destination_storage_id.trim().is_empty() {
        return Err(CoreError::Config(
            "Agent Task publication destination storage is required".into(),
        ));
    }
    if request.outputs.is_empty() || request.outputs.len() > MAX_AGENT_TASK_PUBLISH_FILES {
        return Err(CoreError::Config(format!(
            "Agent Task publication must select between 1 and {MAX_AGENT_TASK_PUBLISH_FILES} outputs"
        )));
    }
    normalize_destination_dir(&request.destination_dir)?;

    let mut outputs = request.outputs.clone();
    outputs.sort_by(|left, right| left.task_path.cmp(&right.task_path));
    let mut seen = HashSet::with_capacity(outputs.len());
    let mut seen_portable = HashSet::with_capacity(outputs.len());
    for output in &outputs {
        validate_output_task_path(&output.task_path)?;
        validate_sha256(&output.sha256)?;
        if !seen.insert(output.task_path.clone())
            || !seen_portable.insert(output.task_path.to_lowercase())
        {
            return Err(CoreError::Config(
                "Agent Task publication contains duplicate or case-colliding output paths".into(),
            ));
        }
    }
    Ok(outputs)
}

fn load_publication_context(
    state: &AppState,
    request: &AgentTaskPublicationRequest,
) -> Result<PublicationContext, CoreError> {
    let workspace = state
        .workspaces
        .find_by_id(&request.workspace_id)?
        .ok_or_else(|| CoreError::Config("Agent Task workspace was not found".into()))?;
    if !workspace_schema_supported(&workspace) {
        return Err(CoreError::Config(
            "Agent Task workspace uses an unsupported schema".into(),
        ));
    }
    if workspace.access_profile != "read_write" {
        return Err(CoreError::Config(
            "Agent Task publication requires the task's read-write Agent Workspace".into(),
        ));
    }

    let workspace_storage = state
        .find_storage_by_id(&workspace.storage_id)
        .map_err(|_| {
            CoreError::Config("Agent Task workspace storage could not be validated".into())
        })?;
    if !workspace_storage.enabled || workspace_storage.read_only {
        return Err(CoreError::Config(
            "Agent Task workspace storage must remain enabled and writable for publication".into(),
        ));
    }
    let workspace_kind = workspace_storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("Agent Task workspace backend is unsupported".into()))?;
    if !matches!(workspace_kind, SourceKind::Local) {
        return Err(CoreError::Config(
            "Agent Task publication currently requires a Local Filesystem task workspace".into(),
        ));
    }
    let workspace_namespace =
        infimount_mcp::storage_namespace::storage_namespace_fingerprint(&workspace_storage)
            .map_err(|_| {
                CoreError::Config("failed to verify Agent Task workspace identity".into())
            })?;
    if workspace_namespace != workspace.storage_namespace_fingerprint {
        return Err(CoreError::Config(
            "Agent Task workspace storage identity changed; recreate the workspace".into(),
        ));
    }

    let destination_storage = state
        .find_storage_by_id(&request.destination_storage_id)
        .map_err(|_| {
            CoreError::Config("publication destination storage could not be found".into())
        })?;
    if !destination_storage.enabled || destination_storage.read_only {
        return Err(CoreError::Config(
            "publication destination storage must be enabled and writable".into(),
        ));
    }
    let destination_namespace =
        infimount_mcp::storage_namespace::storage_namespace_fingerprint(&destination_storage)
            .map_err(|_| {
                CoreError::Config("failed to verify publication destination identity".into())
            })?;

    let workspace_op = state.operator_for_storage_id(&workspace_storage.id)?;
    let workspace_caps = workspace_op.info().capability();
    if !workspace_caps.stat
        || !workspace_caps.read
        || !workspace_caps.write
        || !workspace_caps.write_with_if_not_exists
    {
        return Err(CoreError::Config(
            "Agent Task workspace lacks required safe receipt capabilities for publication".into(),
        ));
    }
    let destination_op = state.operator_for_storage_id(&destination_storage.id)?;
    let destination_caps = destination_op.info().capability();
    if !destination_caps.stat
        || !destination_caps.read
        || !destination_caps.write
        || !destination_caps.write_with_if_not_exists
    {
        return Err(CoreError::Config(
            "publication destination does not support safe create-only publication".into(),
        ));
    }

    let task_root = agent_task_root(&request.task_id)?;
    let workspace_task_path = join_path(&workspace.root_path, &task_root);
    validate_local_path(&workspace_storage, &workspace.root_path)?;
    validate_local_path(&workspace_storage, &workspace_task_path)?;
    let destination_dir = normalize_destination_dir(&request.destination_dir)?;
    validate_destination_path(&destination_storage, &destination_dir)?;

    Ok(PublicationContext {
        workspace,
        workspace_storage,
        destination_storage,
        workspace_op,
        destination_op,
        workspace_namespace,
        destination_namespace,
        task_root,
        workspace_task_path,
        destination_dir,
    })
}

async fn validate_task_manifest(
    context: &PublicationContext,
    request: &AgentTaskPublicationRequest,
) -> Result<(), CoreError> {
    require_directory(
        &context.workspace_op,
        &context.workspace_task_path,
        "Agent Task package",
    )
    .await?;
    let manifest_path = join_path(&context.workspace_task_path, AGENT_TASK_MANIFEST_FILE);
    validate_local_path(&context.workspace_storage, &manifest_path)?;
    let metadata = context
        .workspace_op
        .stat(&manifest_path)
        .await
        .map_err(|_| {
            CoreError::Config("Agent Task manifest could not be read for publication".into())
        })?;
    if !metadata.is_file() || metadata.content_length() > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is missing, invalid, or unexpectedly large".into(),
        ));
    }
    let bytes = context.workspace_op.read(&manifest_path).await?.to_vec();
    if bytes.len() as u64 > MAX_AGENT_TASK_MANIFEST_BYTES {
        return Err(CoreError::Config(
            "Agent Task manifest is unexpectedly large".into(),
        ));
    }
    let manifest: AgentTaskManifest = serde_json::from_slice(&bytes)?;
    validate_agent_task_manifest(&manifest)?;
    if manifest.task_id != request.task_id
        || manifest.workspace_id != request.workspace_id
        || manifest.task_root != context.task_root
    {
        return Err(CoreError::Config(
            "Agent Task manifest does not match the requested publication task".into(),
        ));
    }
    Ok(())
}

fn reject_task_destination_overlap(
    context: &PublicationContext,
    source_path: &str,
    destination_path: &str,
) -> Result<(), CoreError> {
    if (context.workspace_namespace == context.destination_namespace
        && is_reserved_workspace_destination(&context.workspace.root_path, destination_path))
        || destination_aliases_reserved_workspace_control_path(
            &context.workspace_storage,
            &context.workspace.root_path,
            &context.destination_storage,
            destination_path,
        )?
    {
        return Err(CoreError::Config(
            "Agent Task outputs cannot be published into reserved workspace control paths".into(),
        ));
    }
    let relation = infimount_mcp::storage_namespace::transfer_namespace_relation(
        &context.workspace_storage,
        source_path,
        &context.destination_storage,
        destination_path,
    )
    .map_err(|error| CoreError::Config(error.to_string()))?;
    if infimount_mcp::storage_namespace::transfer_has_namespace_conflict(&relation) {
        return Err(CoreError::Config(
            "publication destination overlaps the reviewed Agent Task output".into(),
        ));
    }
    Ok(())
}

fn is_reserved_workspace_destination(workspace_root: &str, destination_path: &str) -> bool {
    let tasks_root = join_path(workspace_root, AGENT_TASKS_DIR).to_lowercase();
    let internal_root = join_path(workspace_root, ".infimount").to_lowercase();
    let destination_path = destination_path.to_lowercase();
    is_path_within(&tasks_root, &destination_path)
        || is_path_within(&internal_root, &destination_path)
}

fn destination_aliases_reserved_workspace_control_path(
    workspace_storage: &StorageRecord,
    workspace_root: &str,
    destination_storage: &StorageRecord,
    destination_path: &str,
) -> Result<bool, CoreError> {
    for reserved_path in [
        join_path(workspace_root, AGENT_TASKS_DIR),
        join_path(workspace_root, ".infimount"),
    ] {
        let relation = infimount_mcp::storage_namespace::transfer_namespace_relation(
            workspace_storage,
            &reserved_path,
            destination_storage,
            destination_path,
        )
        .map_err(|error| CoreError::Config(error.to_string()))?;
        if infimount_mcp::storage_namespace::transfer_has_namespace_conflict(&relation) {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn unique_destination_path(
    op: &opendal::Operator,
    base_path: &str,
) -> Result<String, CoreError> {
    let (parent, name) = split_parent(base_path)?;
    let (stem, extension) = split_file_name(name);
    for index in 1..=9_999u32 {
        let suffix = if index == 1 {
            " copy".to_string()
        } else {
            format!(" copy {index}")
        };
        let candidate = join_path(&parent, &format!("{stem}{suffix}{extension}"));
        if candidate.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES {
            return Err(CoreError::Config(
                "renamed publication destination exceeds the path length limit".into(),
            ));
        }
        if !path_exists(op, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(CoreError::Config(
        "could not choose a non-conflicting publication destination".into(),
    ))
}

async fn ensure_publication_parent(
    op: &opendal::Operator,
    storage: &StorageRecord,
    destination_path: &str,
) -> Result<(), CoreError> {
    let Some((parent, _)) = destination_path.rsplit_once('/') else {
        return Ok(());
    };
    if parent.is_empty() || !op.info().capability().create_dir {
        return Ok(());
    }
    validate_destination_path(storage, parent)?;
    op.create_dir(&format!("{}/", parent.trim_end_matches('/')))
        .await?;
    Ok(())
}

async fn write_publication_receipt(
    op: &opendal::Operator,
    path: &str,
    bytes: &[u8],
) -> Result<(), CoreError> {
    if !op.info().capability().write_with_if_not_exists {
        return Err(CoreError::Config(
            "Agent Task workspace does not support safe create-only publication receipts".into(),
        ));
    }

    let expected = AgentTaskFileDigest {
        byte_size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    };
    let mut writer = op.writer_with(path).if_not_exists(true).await?;
    if let Err(error) = writer.write(bytes.to_vec()).await {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(error.into());
    }
    if let Err(error) = writer.close().await {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(error.into());
    }

    match hash_agent_task_file(op, path).await {
        Ok(actual) if actual == expected => Ok(()),
        Ok(_) => Err(CoreError::TransferCleanupRequired),
        Err(CoreError::Storage(error)) if error.kind() == ErrorKind::NotFound => {
            Err(CoreError::Config(
                "Agent Task publication receipt disappeared during integrity verification".into(),
            ))
        }
        Err(_) => Err(CoreError::TransferCleanupRequired),
    }
}

async fn path_exists(op: &opendal::Operator, path: &str) -> Result<bool, CoreError> {
    match op.stat(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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

fn normalize_destination_dir(path: &str) -> Result<String, CoreError> {
    let normalized = path.trim().trim_matches('/');
    if normalized.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES || normalized.contains('\\') {
        return Err(CoreError::Config(
            "publication destination must be a bounded forward-slash path".into(),
        ));
    }
    if normalized.chars().any(|ch| ch == '\0' || ch.is_control())
        || (!normalized.is_empty()
            && normalized
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == ".."))
    {
        return Err(CoreError::Config(
            "publication destination contains an invalid path segment".into(),
        ));
    }
    Ok(normalized.to_string())
}

fn validate_output_task_path(path: &str) -> Result<(), CoreError> {
    if path.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES
        || !path.starts_with("outputs/")
        || path.len() <= "outputs/".len()
        || path.contains('\\')
        || path.chars().any(|ch| ch == '\0' || ch.is_control())
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(CoreError::Config(
            "Agent Task publication path must be a safe child of outputs/".into(),
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
            "Agent Task publication requires lowercase SHA-256 fingerprints".into(),
        ));
    }
    Ok(())
}

fn validate_preview_token(value: &str) -> Result<(), CoreError> {
    validate_sha256(value)
        .map_err(|_| CoreError::Config("invalid publication preview token".into()))
}

fn validate_local_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    infimount_mcp::storage_namespace::validate_local_mcp_path(storage, path)
        .map_err(|_| CoreError::Config("Agent Task local path confinement check failed".into()))
}

fn validate_destination_path(storage: &StorageRecord, path: &str) -> Result<(), CoreError> {
    let normalized = path.trim_matches('/');
    if normalized.len() > MAX_AGENT_TASK_DESTINATION_PATH_BYTES
        || normalized.contains('\\')
        || normalized.chars().any(|ch| ch == '\0' || ch.is_control())
        || (!normalized.is_empty()
            && normalized
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == ".."))
    {
        return Err(CoreError::Config(
            "publication destination path is invalid or exceeds the path length limit".into(),
        ));
    }

    let kind = storage
        .backend
        .parse::<SourceKind>()
        .map_err(|_| CoreError::Config("publication destination backend is unsupported".into()))?;
    if matches!(kind, SourceKind::Local) && !normalized.is_empty() {
        validate_local_path(storage, normalized)?;
    }
    Ok(())
}

fn is_path_within(root: &str, path: &str) -> bool {
    let root = root.trim_matches('/');
    let path = path.trim_matches('/');
    path == root || path.starts_with(&format!("{root}/"))
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

fn split_parent(path: &str) -> Result<(String, &str), CoreError> {
    let normalized = path.trim_matches('/');
    if normalized.is_empty() {
        return Err(CoreError::Config(
            "publication destination file path is empty".into(),
        ));
    }
    Ok(match normalized.rsplit_once('/') {
        Some((parent, name)) => (parent.to_string(), name),
        None => (String::new(), normalized),
    })
}

fn split_file_name(name: &str) -> (String, String) {
    if name.starts_with('.') {
        return (name.to_string(), String::new());
    }
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem.to_string(), format!(".{extension}"))
        }
        _ => (name.to_string(), String::new()),
    }
}

fn relative_to_root(root: &str, path: &str) -> Result<String, CoreError> {
    let root = root.trim().trim_matches('/');
    let path = path.trim().trim_matches('/');
    if root.is_empty() {
        return (!path.is_empty())
            .then(|| path.to_string())
            .ok_or_else(|| CoreError::Config("publication receipt path is invalid".into()));
    }
    path.strip_prefix(&format!("{root}/"))
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CoreError::Config("publication receipt escaped the workspace".into()))
}

fn sha256_json(value: &impl Serialize) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_paths_reject_traversal_and_windows_separators() {
        assert_eq!(
            normalize_destination_dir("/exports/reports/").unwrap(),
            "exports/reports"
        );
        assert!(normalize_destination_dir("../secret").is_err());
        assert!(normalize_destination_dir("exports\\secret").is_err());
        assert!(validate_output_task_path("outputs/report.md").is_ok());
        assert!(validate_output_task_path("inputs/report.md").is_err());
    }

    #[test]
    fn reserved_workspace_control_paths_are_not_publication_destinations() {
        let workspace = "/agent-workspaces/coding";
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/tasks/new-task/report.md"
        ));
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/TASKS/new-task/report.md"
        ));
        assert!(is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/.INFIMOUNT/checkpoints/fake.json"
        ));
        assert!(!is_reserved_workspace_destination(
            workspace,
            "agent-workspaces/coding/published/report.md"
        ));
        assert!(!is_reserved_workspace_destination(
            workspace,
            "elsewhere/tasks/report.md"
        ));
    }

    #[test]
    fn reserved_control_paths_reject_nested_local_storage_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let storage_root = temp.path().join("storage");
        let workspace_root = "agent-workspaces/coding";
        let tasks_root = storage_root.join(workspace_root).join("tasks");
        let internal_root = storage_root.join(workspace_root).join(".infimount");
        let other_root = temp.path().join("other");
        std::fs::create_dir_all(&tasks_root).unwrap();
        std::fs::create_dir_all(&internal_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();

        let workspace_storage = StorageRecord::new(
            "workspace".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": storage_root.to_string_lossy() }),
        );
        let tasks_alias = StorageRecord::new(
            "tasks-alias".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": tasks_root.to_string_lossy() }),
        );
        let internal_alias = StorageRecord::new(
            "internal-alias".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": internal_root.to_string_lossy() }),
        );
        let unrelated = StorageRecord::new(
            "unrelated".to_string(),
            "local".to_string(),
            serde_json::json!({ "root": other_root.to_string_lossy() }),
        );

        assert!(destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &tasks_alias,
            "published/report.md",
        )
        .unwrap());
        assert!(destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &internal_alias,
            "checkpoints/fake.json",
        )
        .unwrap());
        assert!(!destination_aliases_reserved_workspace_control_path(
            &workspace_storage,
            workspace_root,
            &unrelated,
            "published/report.md",
        )
        .unwrap());
    }

    #[tokio::test]
    async fn publication_receipts_are_create_only_and_integrity_checked() {
        let op = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
        assert!(op.info().capability().write_with_if_not_exists);
        let path = "tasks/task/publish-receipt-publication.json";
        let bytes = b"{\"publicationId\":\"publication\"}\n";

        write_publication_receipt(&op, path, bytes).await.unwrap();
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
        assert!(write_publication_receipt(&op, path, bytes).await.is_err());
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
    }

    #[test]
    fn publication_receipt_does_not_contain_source_storage_or_host_paths() {
        let receipt = AgentTaskPublishReceipt {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            publication_id: "publication-id".into(),
            task_id: "task-id".into(),
            workspace_id: "workspace-id".into(),
            published_at: "2026-09-08T00:00:00Z".into(),
            destination_storage_id: "destination-id".into(),
            destination_storage_name: "Published".into(),
            destination_dir: "exports".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Fail,
            files: vec![AgentTaskPublicationPlanFile {
                task_path: "outputs/report.md".into(),
                byte_size: 3,
                sha256: "a".repeat(64),
                destination_path: "exports/report.md".into(),
                action: AgentTaskPublishAction::Create,
            }],
        };
        let json = serde_json::to_value(receipt).unwrap();
        assert!(json.get("sourceStorageId").is_none());
        assert!(json.get("sourcePath").is_none());
        assert!(json.get("hostPath").is_none());
    }

    #[test]
    fn preview_token_binds_destination_and_reviewed_hashes() {
        let files = vec![AgentTaskPublicationPlanFile {
            task_path: "outputs/report.md".into(),
            byte_size: 3,
            sha256: "a".repeat(64),
            destination_path: "exports/report.md".into(),
            action: AgentTaskPublishAction::Create,
        }];
        let fingerprint = AgentTaskPublicationFingerprint {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            workspace_id: "workspace",
            task_id: "task",
            workspace_namespace: "workspace-ns",
            destination_storage_id: "destination",
            destination_namespace: "destination-ns",
            destination_dir: "exports",
            conflict_policy: &AgentTaskPublishConflictPolicy::Fail,
            files: &files,
        };
        let token = sha256_json(&fingerprint).unwrap();
        let mut changed = files.clone();
        changed[0].sha256 = "b".repeat(64);
        let changed_fingerprint = AgentTaskPublicationFingerprint {
            files: &changed,
            ..fingerprint
        };
        assert_ne!(token, sha256_json(&changed_fingerprint).unwrap());
    }
}
