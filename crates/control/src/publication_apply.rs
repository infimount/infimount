//! Shared, integrity-checked publication apply primitives.
//!
//! These helpers are privileged storage mechanics, NOT authorization. A
//! fingerprint is a stale-state detector, not a human approval, capability
//! token or permission to write. Only a trusted caller holding the lifecycle,
//! configuration and workspace mutation locks may apply a rebuilt plan.
//! Headless CLI/IPC must add server-side, scoped, single-use approval before
//! calling any mutating operation.
use infimount_core::agent_task_io::{
    hash_agent_task_file, publish_reviewed_agent_task_file, AgentTaskFileDigest,
};
use infimount_core::CoreError;
use infimount_mcp::registry::StorageRecord;
use opendal::{ErrorKind, Operator};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::publication::{
    normalize_destination_dir, validate_publication_request, AgentTaskPublicationRequest,
};
use crate::publication_context::{validate_destination_path, validate_local_path};
use crate::publication_engine::PublicationPlan;
use crate::publication_plan::{
    publication_preview_fingerprint, AgentTaskPublicationFingerprint, AgentTaskPublicationPlanFile,
    AgentTaskPublishReceipt, AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
};

const AGENT_TASK_PUBLISH_RECEIPT_PREFIX: &str = "publish-receipt-";

/// The existing desktop publication response, retained byte-for-byte at
/// the serialized camelCase boundary while the trusted apply moves here.
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

/// Validate a freshly rebuilt, trusted plan against an earlier preview.
/// This must execute UNDER the existing desktop publication mutation locks.
/// It intentionally does not confer permission to apply.
pub fn validate_locked_publication_plan(
    current_preview_token: &str,
    reviewed_preview_token: &str,
    conflict_count: usize,
) -> Result<(), CoreError> {
    if current_preview_token != reviewed_preview_token {
        return Err(CoreError::Config(
            "Agent Task publication changed since review; review the publication plan again".into(),
        ));
    }
    if conflict_count != 0 {
        return Err(CoreError::Config(
            "Agent Task publication has unresolved destination conflicts".into(),
        ));
    }
    Ok(())
}

/// Only create necessary parent directories when supported. The caller must
/// validate the full destination path and namespace/alias safety first.
pub async fn ensure_publication_parent(
    op: &Operator,
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

/// Persist a receipt only if it doesn't already exist, then verify the exact
/// bytes. Never overwrite or opportunistically delete ambiguous state.
pub async fn write_publication_receipt(
    op: &Operator,
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

/// Commit a freshly rebuilt publication plan using only create-if-missing
/// writes and verified source/destination digests. The caller MUST already
/// hold the lifecycle/configuration/workspace mutation locks and, for any
/// headless caller, an atomically consumed server-held human approval.
///
/// This check and the byte copy are co-located so no consumer can forget
/// stale-preview and conflict validation when invoking this shared loop.
/// This function is a privileged Rust primitive, not an agent-facing API.
pub(crate) async fn publish_locked_reviewed_files(
    plan: &PublicationPlan,
    reviewed_preview_token: &str,
) -> Result<Vec<AgentTaskPublicationPlanFile>, CoreError> {
    validate_locked_publication_plan(
        &plan.preview_token,
        reviewed_preview_token,
        plan.conflict_count,
    )?;

    let mut published = Vec::<AgentTaskPublicationPlanFile>::new();
    for file in &plan.files {
        let source_path = join_path(&plan.context.workspace_task_path, &file.task_path);
        let copy_result = async {
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
            publish_reviewed_agent_task_file(
                &plan.context.workspace_op,
                &source_path,
                &plan.context.destination_op,
                &file.destination_path,
                &expected,
            )
            .await?;
            Ok::<(), CoreError>(())
        }
        .await;
        if let Err(error) = copy_result {
            if published.is_empty() {
                return Err(error);
            }
            // Any failure after a previous commit, including destination
            // validation or parent creation, needs explicit cleanup. Deleting
            // already committed paths could destroy concurrent writer data.
            return Err(CoreError::TransferCleanupRequired);
        }
        published.push(file.clone());
    }
    Ok(published)
}

/// Complete the desktop-mediated publication transaction, including its
/// unique create-only integrity-checked receipt. The caller MUST hold the
/// lifecycle, configuration and workspace mutation locks throughout.
/// A future headless caller MUST additionally supply an authenticated,
/// atomically consumed server-held human approval. This is not an endpoint.
pub async fn commit_locked_publication<F>(
    plan: &PublicationPlan,
    request: &AgentTaskPublicationRequest,
    reviewed_preview_token: &str,
    issue_receipt_identity: F,
) -> Result<AgentTaskPublicationOutput, CoreError>
where
    F: FnOnce() -> (String, String),
{
    validate_request_matches_locked_plan(plan, request)?;
    let published = publish_locked_reviewed_files(plan, reviewed_preview_token).await?;
    // Issue the receipt identity after the exact reviewed output bytes commit.
    let (publication_id, published_at) = issue_receipt_identity();
    let receipt = AgentTaskPublishReceipt {
        schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
        publication_id: publication_id.clone(),
        task_id: request.task_id.clone(),
        workspace_id: request.workspace_id.clone(),
        published_at: published_at.clone(),
        destination_storage_id: plan.context.destination_storage.id.clone(),
        destination_storage_name: plan.context.destination_storage.name.clone(),
        destination_dir: plan.context.destination_dir.clone(),
        conflict_policy: request.conflict_policy.clone(),
        files: published.clone(),
    };
    let receipt_file = format!("{AGENT_TASK_PUBLISH_RECEIPT_PREFIX}{publication_id}.json");
    let receipt_path = join_path(&plan.context.workspace_task_path, &receipt_file);
    // Any failure after output bytes committed leaves potentially ambiguous
    // destination/receipt state. Never try to rollback by deleting paths.
    let relative_receipt_path = async {
        validate_local_path(&plan.context.workspace_storage, &receipt_path)?;
        let mut bytes = serde_json::to_vec_pretty(&receipt)?;
        bytes.push(b'\n');
        write_publication_receipt(&plan.context.workspace_op, &receipt_path, &bytes).await?;
        relative_to_root(&plan.context.workspace.root_path, &receipt_path)
    }
    .await
    .map_err(|_| CoreError::TransferCleanupRequired)?;

    Ok(AgentTaskPublicationOutput {
        publication_id,
        published_at,
        workspace_id: request.workspace_id.clone(),
        task_id: request.task_id.clone(),
        destination_storage_id: plan.context.destination_storage.id.clone(),
        destination_storage_name: plan.context.destination_storage.name.clone(),
        receipt_path: relative_receipt_path,
        files: published,
    })
}

fn validate_request_matches_locked_plan(
    plan: &PublicationPlan,
    request: &AgentTaskPublicationRequest,
) -> Result<(), CoreError> {
    let selected = validate_publication_request(request)?;
    let recomputed_preview_token = fingerprint_for_request(plan, request)?;
    let task_id = plan
        .context
        .task_root
        .strip_prefix("tasks/")
        .ok_or_else(|| CoreError::Config("trusted publication task binding is invalid".into()))?;
    if request.workspace_id != plan.context.workspace.id
        || request.task_id != task_id
        || request.destination_storage_id != plan.context.destination_storage.id
        || normalize_destination_dir(&request.destination_dir)? != plan.context.destination_dir
        || plan.preview_token != recomputed_preview_token
        || selected.len() != plan.files.len()
        || selected.iter().zip(&plan.files).any(|(reviewed, planned)| {
            reviewed.task_path != planned.task_path
                || reviewed.byte_size != planned.byte_size
                || reviewed.sha256 != planned.sha256
        })
    {
        return Err(CoreError::Config(
            "reviewed publication request no longer matches the trusted plan".into(),
        ));
    }
    Ok(())
}

fn fingerprint_for_request(
    plan: &PublicationPlan,
    request: &AgentTaskPublicationRequest,
) -> Result<String, CoreError> {
    publication_preview_fingerprint(&AgentTaskPublicationFingerprint {
        schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
        workspace_id: &request.workspace_id,
        task_id: &request.task_id,
        workspace_namespace: &plan.context.workspace_namespace,
        destination_storage_id: &plan.context.destination_storage.id,
        destination_namespace: &plan.context.destination_namespace,
        destination_dir: &plan.context.destination_dir,
        conflict_policy: &request.conflict_policy,
        files: &plan.files,
    })
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
    use serde_json::json;

    async fn plan_fixture() -> (tempfile::TempDir, PublicationPlan) {
        use crate::publication_plan::AgentTaskPublishAction;
        use infimount_core::workspaces::{WorkspaceRecord, WORKSPACE_RECORD_SCHEMA_VERSION};
        use infimount_mcp::storage_namespace::storage_namespace_fingerprint;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let mut source = StorageRecord::new("Source".into(), "local".into(), json!({"root": root}));
        source.id = "source".into();
        let mut destination =
            StorageRecord::new("Published".into(), "local".into(), json!({"root": root}));
        destination.id = "destination".into();
        let namespace = storage_namespace_fingerprint(&source).unwrap();
        let workspace = WorkspaceRecord {
            id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: source.id.clone(),
            name: "Coding".into(),
            root_path: "workspaces/coding".into(),
            template_id: "coding".into(),
            access_profile: "read_write".into(),
            policy_rule_id: None,
            storage_namespace_fingerprint: namespace.clone(),
            created_at: String::new(),
            updated_at: String::new(),
            memory_files: Vec::new(),
            checkpoint_ids: Vec::new(),
        };
        let op = Operator::new(opendal::services::Fs::default().root(&root)).unwrap();
        let task_root = "tasks/81f08176-86e4-40ec-a9a4-a219c4c9b454";
        let task_path = format!("workspaces/coding/{task_root}");
        let outputs_dir = format!("{task_path}/outputs/");
        op.create_dir(&outputs_dir).await.unwrap();
        let mut files = Vec::new();
        for (name, contents) in [("a.txt", "alpha"), ("b.txt", "bravo")] {
            let source_path = format!("{outputs_dir}{name}");
            op.write(&source_path, contents).await.unwrap();
            let digest = hash_agent_task_file(&op, &source_path).await.unwrap();
            files.push(AgentTaskPublicationPlanFile {
                task_path: format!("outputs/{name}"),
                byte_size: digest.byte_size,
                sha256: digest.sha256,
                destination_path: format!("exports/{name}"),
                action: AgentTaskPublishAction::Create,
            });
        }
        let context = crate::publication_context::PublicationContext {
            workspace,
            workspace_storage: source,
            destination_storage: destination,
            workspace_op: op.clone(),
            destination_op: op,
            workspace_namespace: namespace.clone(),
            destination_namespace: namespace,
            task_root: task_root.into(),
            workspace_task_path: task_path,
            destination_dir: "exports".into(),
        };
        (
            temp,
            PublicationPlan {
                context,
                files,
                preview_token: "a".repeat(64),
                total_bytes: 10,
                create_count: 2,
                rename_count: 0,
                conflict_count: 0,
            },
        )
    }

    #[tokio::test]
    async fn locked_publication_creates_and_verifies_reviewed_bytes() {
        let (_temp, plan) = plan_fixture().await;
        let published = publish_locked_reviewed_files(&plan, &plan.preview_token)
            .await
            .unwrap();
        assert_eq!(published, plan.files);
        assert_eq!(
            plan.context
                .destination_op
                .read("exports/a.txt")
                .await
                .unwrap()
                .to_vec(),
            b"alpha"
        );
        assert_eq!(
            plan.context
                .destination_op
                .read("exports/b.txt")
                .await
                .unwrap()
                .to_vec(),
            b"bravo"
        );
    }

    #[tokio::test]
    async fn stale_or_conflicting_plan_never_opens_destination_writer() {
        let (_temp, mut plan) = plan_fixture().await;
        let error = publish_locked_reviewed_files(&plan, &"b".repeat(64))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed since review"));
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());

        plan.conflict_count = 1;
        let error = publish_locked_reviewed_files(&plan, &plan.preview_token)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("unresolved destination conflicts"));
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn changed_same_size_input_blocks_create_only_publishing() {
        let (_temp, plan) = plan_fixture().await;
        plan.context
            .workspace_op
            .write(
                &format!("{}/outputs/a.txt", plan.context.workspace_task_path),
                "ALPHA",
            )
            .await
            .unwrap();
        assert!(publish_locked_reviewed_files(&plan, &plan.preview_token)
            .await
            .is_err());
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn late_second_destination_conflict_retains_partial_output_and_sentinel() {
        let (_temp, plan) = plan_fixture().await;
        plan.context
            .destination_op
            .write("exports/b.txt", "sentinel")
            .await
            .unwrap();
        let error = publish_locked_reviewed_files(&plan, &plan.preview_token)
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::TransferCleanupRequired));
        let first = plan
            .context
            .destination_op
            .read("exports/a.txt")
            .await
            .unwrap();
        assert_eq!(first.to_vec(), b"alpha");
        let sentinel = plan
            .context
            .destination_op
            .read("exports/b.txt")
            .await
            .unwrap();
        assert_eq!(sentinel.to_vec(), b"sentinel");
    }

    #[tokio::test]
    async fn second_parent_creation_failure_requires_cleanup_after_first_commit() {
        let (_temp, mut plan) = plan_fixture().await;
        plan.files[1].destination_path = "exports/occupied/b.txt".into();
        plan.context
            .destination_op
            .write("exports/occupied", "sentinel")
            .await
            .unwrap();

        let error = publish_locked_reviewed_files(&plan, &plan.preview_token)
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::TransferCleanupRequired));
        assert_eq!(
            plan.context
                .destination_op
                .read("exports/a.txt")
                .await
                .unwrap()
                .to_vec(),
            b"alpha"
        );
        assert_eq!(
            plan.context
                .destination_op
                .read("exports/occupied")
                .await
                .unwrap()
                .to_vec(),
            b"sentinel"
        );
    }

    fn matching_request(plan: &PublicationPlan) -> AgentTaskPublicationRequest {
        use crate::publication::{AgentTaskPublishConflictPolicy, ReviewedAgentTaskOutput};
        AgentTaskPublicationRequest {
            workspace_id: plan.context.workspace.id.clone(),
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            outputs: plan
                .files
                .iter()
                .map(|file| ReviewedAgentTaskOutput {
                    task_path: file.task_path.clone(),
                    byte_size: file.byte_size,
                    sha256: file.sha256.clone(),
                })
                .collect(),
            destination_storage_id: plan.context.destination_storage.id.clone(),
            destination_dir: "exports".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Fail,
        }
    }

    fn bind_plan_fingerprint(plan: &mut PublicationPlan, request: &AgentTaskPublicationRequest) {
        plan.preview_token = fingerprint_for_request(plan, request).unwrap();
    }

    #[tokio::test]
    async fn full_locked_commit_returns_legacy_wire_and_verifies_receipt() {
        let (_temp, mut plan) = plan_fixture().await;
        let request = matching_request(&plan);
        bind_plan_fingerprint(&mut plan, &request);
        let result = commit_locked_publication(&plan, &request, &plan.preview_token, || {
            (
                "d2534279-250e-4788-a807-c390c2c173b1".into(),
                "2026-10-25T11:00:00+05:30".into(),
            )
        })
        .await
        .unwrap();

        assert_eq!(result.files, plan.files);
        assert_eq!(result.task_id, request.task_id);
        assert_eq!(
            serde_json::to_value(&result).unwrap()["destinationStorageId"],
            "destination"
        );
        let absolute_receipt = join_path(&plan.context.workspace.root_path, &result.receipt_path);
        let receipt_bytes = plan
            .context
            .workspace_op
            .read(&absolute_receipt)
            .await
            .unwrap()
            .to_vec();
        assert!(receipt_bytes.ends_with(b"\n"));
        let receipt: serde_json::Value = serde_json::from_slice(&receipt_bytes).unwrap();
        assert_eq!(receipt["schemaVersion"], 1);
        assert_eq!(receipt["publicationId"], result.publication_id);
        assert_eq!(receipt["files"][0]["destinationPath"], "exports/a.txt");
        assert!(receipt.get("sourceStorageId").is_none());
        assert!(receipt.get("hostPath").is_none());
    }

    #[tokio::test]
    async fn inconsistent_request_rejects_before_any_destination_write() {
        let (_temp, mut plan) = plan_fixture().await;
        let mut request = matching_request(&plan);
        bind_plan_fingerprint(&mut plan, &request);
        request.outputs[0].sha256 = "c".repeat(64);
        let result = commit_locked_publication(&plan, &request, &plan.preview_token, || {
            ("unused".into(), "unused".into())
        })
        .await;
        assert!(result.is_err());
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn altered_conflict_policy_rejects_before_destination_write() {
        use crate::publication::AgentTaskPublishConflictPolicy;
        let (_temp, mut plan) = plan_fixture().await;
        let mut request = matching_request(&plan);
        bind_plan_fingerprint(&mut plan, &request);
        request.conflict_policy = AgentTaskPublishConflictPolicy::Rename;
        let result = commit_locked_publication(&plan, &request, &plan.preview_token, || {
            ("unused".into(), "unused".into())
        })
        .await;
        assert!(result.is_err());
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn destination_binding_change_never_issues_receipt_or_writes() {
        let (_temp, mut plan) = plan_fixture().await;
        let mut request = matching_request(&plan);
        bind_plan_fingerprint(&mut plan, &request);
        request.destination_storage_id = "different".into();
        let result = commit_locked_publication(&plan, &request, &plan.preview_token, || {
            panic!("rejected publication must not issue receipt metadata")
        })
        .await;
        assert!(result.is_err());
        assert!(plan
            .context
            .destination_op
            .stat("exports/a.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn receipt_collision_after_committed_files_requires_cleanup() {
        let (_temp, mut plan) = plan_fixture().await;
        let request = matching_request(&plan);
        bind_plan_fingerprint(&mut plan, &request);
        let receipt_id = "e1b6a197-31a9-4cda-a327-068d413bc4db";
        let receipt = join_path(
            &plan.context.workspace_task_path,
            &format!("publish-receipt-{receipt_id}.json"),
        );
        plan.context
            .workspace_op
            .write(&receipt, "sentinel")
            .await
            .unwrap();
        let error = commit_locked_publication(&plan, &request, &plan.preview_token, || {
            (receipt_id.into(), "2026-10-25T11:00:00+05:30".into())
        })
        .await
        .unwrap_err();
        assert!(matches!(error, CoreError::TransferCleanupRequired));
        assert_eq!(
            plan.context
                .destination_op
                .read("exports/a.txt")
                .await
                .unwrap()
                .to_vec(),
            b"alpha"
        );
        assert_eq!(
            plan.context
                .workspace_op
                .read(&receipt)
                .await
                .unwrap()
                .to_vec(),
            b"sentinel"
        );
    }

    #[test]
    fn stale_plan_rejected_and_conflict_has_distinct_error() {
        assert!(validate_locked_publication_plan("current", "previous", 0)
            .unwrap_err()
            .to_string()
            .contains("changed since review"));
        assert!(validate_locked_publication_plan("current", "current", 1)
            .unwrap_err()
            .to_string()
            .contains("unresolved destination conflicts"));
        assert!(validate_locked_publication_plan("current", "current", 0).is_ok());
    }

    #[tokio::test]
    async fn receipts_are_create_only_exactly_verified_and_replay_fails_closed() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        assert!(op.info().capability().write_with_if_not_exists);
        let path = "tasks/task/publish-receipt-publication.json";
        let bytes = b"{\"publicationId\":\"publication\"}\n";
        write_publication_receipt(&op, path, bytes).await.unwrap();
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
        assert!(write_publication_receipt(&op, path, bytes).await.is_err());
        let changed = b"{\"publicationId\":\"changed\"}\n";
        assert!(write_publication_receipt(&op, path, changed).await.is_err());
        assert_eq!(op.read(path).await.unwrap().to_vec(), bytes);
    }

    #[tokio::test]
    async fn parent_creation_is_capability_gated_and_rejects_traversal() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        let storage = StorageRecord::new("Remote".into(), "s3".into(), json!({}));
        ensure_publication_parent(&op, &storage, "reports/2026/summary.md")
            .await
            .unwrap();
        assert!(op.stat("reports/2026/").await.unwrap().is_dir());
        assert!(
            ensure_publication_parent(&op, &storage, "reports/../secret/report.md")
                .await
                .is_err()
        );
        assert!(
            ensure_publication_parent(&op, &storage, "reports\\secret/report.md")
                .await
                .is_err()
        );
    }
}
