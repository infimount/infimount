//! Trusted, read-only Agent Task publication planning used by the desktop.
//!
//! This engine validates the task manifest, current source hashes, destination
//! collisions and namespace overlap, then produces a staleness fingerprint.
//! It does not grant approval or write any object. The desktop still owns
//! configuration/workspace mutation locks, create-only writes and receipts.
use std::collections::HashSet;

use infimount_core::agent_tasks::{
    validate_agent_task_manifest, AgentTaskManifest, AGENT_TASKS_DIR, AGENT_TASK_MANIFEST_FILE,
};
use infimount_core::workspaces::WorkspaceRecord;
use infimount_core::CoreError;
use infimount_mcp::registry::StorageRecord;
use opendal::Operator;

use crate::publication::{
    validate_publication_request, AgentTaskPublicationRequest,
};
use crate::publication_context::{
    resolve_publication_context, validate_destination_path, validate_local_path, PublicationContext,
};
use crate::publication_destination::{
    choose_publication_destination, reserve_publication_destination_key,
};
use crate::publication_evidence::verify_reviewed_output_digest;
use crate::publication_plan::{
    publication_preview_fingerprint, AgentTaskPublicationFingerprint, AgentTaskPublicationPlanFile,
    AgentTaskPublishAction, AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
};

const MAX_AGENT_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AGENT_TASK_PUBLISH_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// All fields are trusted internal state; not serialized into agent-visible
/// context, and never accepted as inputs to a publication apply endpoint.
pub struct PublicationPlan {
    pub context: PublicationContext,
    pub files: Vec<AgentTaskPublicationPlanFile>,
    pub preview_token: String,
    pub total_bytes: u64,
    pub create_count: usize,
    pub rename_count: usize,
    pub conflict_count: usize,
}

/// Build a preview from verified task bytes and current destination state.
/// A fingerprint is a stale-state marker, never a capability or approval.
/// The caller must own the authoritative mutation locks when applying writes.
pub async fn build_publication_plan<FindWorkspace, FindStorage, OpenStorage>(
    request: &AgentTaskPublicationRequest,
    find_workspace: FindWorkspace,
    find_storage: FindStorage,
    open_storage: OpenStorage,
) -> Result<PublicationPlan, CoreError>
where
    FindWorkspace: FnOnce(&str) -> Result<Option<WorkspaceRecord>, CoreError>,
    FindStorage: FnMut(&str) -> Result<StorageRecord, CoreError>,
    OpenStorage: FnMut(&str) -> Result<Operator, CoreError>,
{
    let outputs = validate_publication_request(request)?;
    let context = resolve_publication_context(
        request,
        find_workspace,
        find_storage,
        open_storage,
    )?;
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
        let current =
            verify_reviewed_output_digest(&context.workspace_op, &source_path, &reviewed).await?;
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

        let (destination_path, action) = choose_publication_destination(
            &context.destination_op,
            &base_destination,
            &request.conflict_policy,
        )
        .await?;
        validate_destination_path(&context.destination_storage, &destination_path)?;
        reject_task_destination_overlap(&context, &source_path, &destination_path)?;
        reserve_publication_destination_key(&mut destination_keys, &destination_path)?;

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
    let preview_token = publication_preview_fingerprint(&fingerprint)?;

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

pub fn is_reserved_workspace_destination(workspace_root: &str, destination_path: &str) -> bool {
    let tasks_root = join_path(workspace_root, AGENT_TASKS_DIR).to_lowercase();
    let internal_root = join_path(workspace_root, ".infimount").to_lowercase();
    let destination_path = destination_path.to_lowercase();
    is_path_within(&tasks_root, &destination_path)
        || is_path_within(&internal_root, &destination_path)
}

pub fn destination_aliases_reserved_workspace_control_path(
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

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_task_io::hash_agent_task_file;
    use infimount_core::agent_tasks::{agent_task_root, AGENT_TASK_SCHEMA_VERSION};
    use infimount_core::workspaces::WORKSPACE_RECORD_SCHEMA_VERSION;
    use infimount_mcp::storage_namespace::storage_namespace_fingerprint;
    use serde_json::json;

    const WORKSPACE_ID: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";
    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn storage(name: &str, id: &str, root: &str) -> StorageRecord {
        let mut record =
            StorageRecord::new(name.into(), "local".into(), json!({"root": root}));
        record.id = id.into();
        record
    }

    fn workspace(record: &StorageRecord) -> WorkspaceRecord {
        WorkspaceRecord {
            id: WORKSPACE_ID.into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: record.id.clone(),
            name: "Coding".into(),
            root_path: "workspaces/coding".into(),
            template_id: "coding".into(),
            access_profile: "read_write".into(),
            policy_rule_id: None,
            storage_namespace_fingerprint: storage_namespace_fingerprint(record).unwrap(),
            created_at: String::new(),
            updated_at: String::new(),
            memory_files: Vec::new(),
            checkpoint_ids: Vec::new(),
        }
    }

    async fn fixture() -> (
        tempfile::TempDir,
        Operator,
        WorkspaceRecord,
        StorageRecord,
        StorageRecord,
        AgentTaskPublicationRequest,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let source = storage("Workspace", "source", &root);
        let destination = storage("Destination", "destination", &root);
        let workspace = workspace(&source);
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        let task_root = agent_task_root(TASK_ID).unwrap();
        let package_path = format!("workspaces/coding/{task_root}/");
        let outputs_path = format!("{package_path}outputs/");
        op.create_dir(&package_path).await.unwrap();
        op.create_dir(&outputs_path).await.unwrap();
        let manifest = AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: TASK_ID.into(),
            title: "Reviewed report".into(),
            created_at: "2026-10-23T00:00:00Z".into(),
            workspace_id: WORKSPACE_ID.into(),
            task_root,
            outputs_directory: "outputs".into(),
            inputs: Vec::new(),
        };
        op.write(
            &format!("{package_path}{AGENT_TASK_MANIFEST_FILE}"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .await
        .unwrap();
        let file_path = format!("{outputs_path}report.md");
        op.write(&file_path, "hello").await.unwrap();
        let digest = hash_agent_task_file(&op, &file_path).await.unwrap();
        let request = AgentTaskPublicationRequest {
            workspace_id: WORKSPACE_ID.into(),
            task_id: TASK_ID.into(),
            outputs: vec![crate::publication::ReviewedAgentTaskOutput {
                task_path: "outputs/report.md".into(),
                byte_size: digest.byte_size,
                sha256: digest.sha256,
            }],
            destination_storage_id: destination.id.clone(),
            destination_dir: "exports".into(),
            conflict_policy: crate::publication::AgentTaskPublishConflictPolicy::Fail,
        };
        (tmp, op, workspace, source, destination, request)
    }

    async fn plan(
        op: &Operator,
        workspace: &WorkspaceRecord,
        source: &StorageRecord,
        destination: &StorageRecord,
        request: &AgentTaskPublicationRequest,
    ) -> Result<PublicationPlan, CoreError> {
        build_publication_plan(
            request,
            |_| Ok(Some(workspace.clone())),
            |id| {
                if id == source.id {
                    Ok(source.clone())
                } else if id == destination.id {
                    Ok(destination.clone())
                } else {
                    Err(CoreError::Config("unknown storage".into()))
                }
            },
            |_| Ok(op.clone()),
        )
        .await
    }

    #[tokio::test]
    async fn preview_is_read_only_and_fingerprint_reacts_to_source_drift() {
        let (_tmp, op, workspace, source, destination, mut request) = fixture().await;
        let first = plan(&op, &workspace, &source, &destination, &request)
            .await
            .unwrap();
        assert_eq!(first.files.len(), 1);
        assert_eq!(first.files[0].destination_path, "exports/report.md");
        assert_eq!(first.files[0].action, AgentTaskPublishAction::Create);
        assert_eq!(first.total_bytes, 5);
        assert_eq!(first.create_count, 1);
        assert_eq!(first.rename_count, 0);
        assert_eq!(first.conflict_count, 0);
        assert!(op.stat("exports/report.md").await.is_err());

        let file_path =
            format!("workspaces/coding/tasks/{TASK_ID}/outputs/report.md");
        op.write(&file_path, "world").await.unwrap();
        assert!(plan(&op, &workspace, &source, &destination, &request)
            .await
            .is_err());
        request.outputs[0].sha256 =
            hash_agent_task_file(&op, &file_path).await.unwrap().sha256;
        let refreshed = plan(&op, &workspace, &source, &destination, &request)
            .await
            .unwrap();
        assert_ne!(first.preview_token, refreshed.preview_token);
    }

    #[tokio::test]
    async fn destination_conflicts_and_rename_keep_policy_and_do_not_write() {
        let (_tmp, op, workspace, source, destination, mut request) = fixture().await;
        op.write("exports/report.md", "sentinel").await.unwrap();
        let conflict = plan(&op, &workspace, &source, &destination, &request)
            .await
            .unwrap();
        assert_eq!(conflict.conflict_count, 1);
        assert_eq!(conflict.files[0].action, AgentTaskPublishAction::Conflict);
        request.conflict_policy = crate::publication::AgentTaskPublishConflictPolicy::Rename;
        let renamed = plan(&op, &workspace, &source, &destination, &request)
            .await
            .unwrap();
        assert_eq!(renamed.rename_count, 1);
        assert_eq!(renamed.files[0].destination_path, "exports/report copy.md");
        assert_eq!(op.read("exports/report.md").await.unwrap().to_vec(), b"sentinel");
        assert!(op.stat("exports/report copy.md").await.is_err());
    }

    #[tokio::test]
    async fn manifest_mismatch_and_reserved_path_fail_closed() {
        let (_tmp, op, workspace, source, destination, mut request) = fixture().await;
        request.destination_dir = "workspaces/coding/tasks".into();
        assert!(plan(&op, &workspace, &source, &destination, &request)
            .await
            .is_err());
        request.destination_dir = "exports".into();
        let manifest_path =
            format!("workspaces/coding/tasks/{TASK_ID}/{AGENT_TASK_MANIFEST_FILE}");
        op.write(&manifest_path, b"{}".to_vec()).await.unwrap();
        assert!(plan(&op, &workspace, &source, &destination, &request)
            .await
            .is_err());
    }
}
