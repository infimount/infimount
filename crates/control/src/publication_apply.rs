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
use sha2::{Digest, Sha256};

use crate::publication_context::{validate_destination_path, validate_local_path};
use crate::publication_engine::PublicationPlan;
use crate::publication_plan::AgentTaskPublicationPlanFile;

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
pub async fn publish_locked_reviewed_files(
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
            id: "workspace-id".into(),
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
