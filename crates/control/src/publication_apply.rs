//! Shared, integrity-checked publication apply primitives.
//!
//! These helpers are privileged storage mechanics, NOT authorization. A
//! fingerprint is a stale-state detector, not a human approval, capability
//! token or permission to write. Only a trusted caller holding the lifecycle,
//! configuration and workspace mutation locks may apply a rebuilt plan.
//! Headless CLI/IPC must add server-side, scoped, single-use approval before
//! calling any mutating operation.
use infimount_core::agent_task_io::{hash_agent_task_file, AgentTaskFileDigest};
use infimount_core::CoreError;
use infimount_mcp::registry::StorageRecord;
use opendal::{ErrorKind, Operator};
use sha2::{Digest, Sha256};

use crate::publication_context::validate_destination_path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
