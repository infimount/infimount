use futures::io::AsyncReadExt;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{CoreError, Result};
use crate::operations::{
    TransferConflictPolicy, TransferOperation, TransferPlan, TransferPlanAction, TransferPlanEntry,
};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskFileDigest {
    pub byte_size: u64,
    pub sha256: String,
}

/// Hash the exact bytes currently stored at `path` without loading the file
/// into memory. Size is checked before and after the read so a concurrent
/// truncate/extend is rejected. The digest is provenance only; callers that
/// later approve publication must hash again immediately before writing.
pub async fn hash_agent_task_file(op: &Operator, path: &str) -> Result<AgentTaskFileDigest> {
    let normalized = normalize_file_path(path)?;
    let before = op.stat(&normalized).await?;
    if before.is_dir() {
        return Err(CoreError::Config(
            "Agent Task hash path must refer to a file".to_string(),
        ));
    }
    let expected_size = before.content_length();
    let mut reader = op
        .reader(&normalized)
        .await?
        .into_futures_async_read(0..expected_size)
        .await?;

    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut read_bytes = 0_u64;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        read_bytes = read_bytes.saturating_add(read as u64);
        if read_bytes > expected_size {
            return Err(CoreError::Config(
                "Agent Task file changed while being hashed".to_string(),
            ));
        }
        digest.update(&buffer[..read]);
    }

    let after = op.stat(&normalized).await?;
    if after.is_dir() || after.content_length() != expected_size || read_bytes != expected_size {
        return Err(CoreError::Config(
            "Agent Task file changed while being hashed".to_string(),
        ));
    }

    Ok(AgentTaskFileDigest {
        byte_size: read_bytes,
        sha256: format!("{:x}", digest.finalize()),
    })
}

/// Execute an already-validated Agent Task transfer plan without discovering
/// the source tree again. Only the entries present in `plan` are created.
///
/// Agent Task preparation uses this after applying its file-count, byte-size,
/// namespace, and path-collision checks to the plan. A file or directory added
/// to a selected source directory after planning is therefore not copied and
/// cannot bypass those bounds. Planned source entries are re-statted before
/// execution so removals, type changes, and file-size changes fail closed.
pub async fn copy_agent_task_plan(
    from_op: &Operator,
    to_op: &Operator,
    plan: &TransferPlan,
) -> Result<()> {
    if plan.operation != TransferOperation::Copy
        || plan.conflict_policy != TransferConflictPolicy::Fail
        || plan
            .entries
            .iter()
            .any(|entry| entry.action != TransferPlanAction::Create)
    {
        return Err(CoreError::Config(
            "Agent Task copy requires an all-create copy/fail transfer plan".to_string(),
        ));
    }

    let mut directories = plan
        .entries
        .iter()
        .filter(|entry| entry.is_dir)
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        path_depth(&left.destination_path)
            .cmp(&path_depth(&right.destination_path))
            .then_with(|| left.destination_path.cmp(&right.destination_path))
    });

    for entry in directories {
        ensure_planned_source(entry, from_op).await?;
        require_missing_destination(to_op, &entry.destination_path).await?;
        to_op.create_dir(&entry.destination_path).await?;
    }

    let mut files = plan
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.destination_path.cmp(&right.destination_path));

    for entry in files {
        ensure_planned_source(entry, from_op).await?;
        require_missing_destination(to_op, &entry.destination_path).await?;
        let copied = copy_agent_task_file(
            from_op,
            &entry.source_path,
            to_op,
            &entry.destination_path,
            entry.size,
        )
        .await?;
        if copied.byte_size != entry.size {
            return Err(CoreError::Config(
                "Agent Task source changed after planning".to_string(),
            ));
        }
    }

    Ok(())
}

/// Copy one file between OpenDAL operators while hashing the bytes actually
/// copied. The source must still match the byte size from the authoritative
/// transfer plan before the destination writer is opened, and it is checked
/// again after the stream. The caller must ensure the destination does not
/// exist before invoking this function.
async fn copy_agent_task_file(
    from_op: &Operator,
    from_path: &str,
    to_op: &Operator,
    to_path: &str,
    expected_size: u64,
) -> Result<AgentTaskFileDigest> {
    let from_path = normalize_file_path(from_path)?;
    let to_path = normalize_file_path(to_path)?;
    let before = from_op.stat(&from_path).await?;
    if before.is_dir() {
        return Err(CoreError::Config(
            "Agent Task copy source must refer to a file".to_string(),
        ));
    }
    if before.content_length() != expected_size {
        return Err(CoreError::Config(
            "Agent Task source changed after planning".to_string(),
        ));
    }

    let mut reader = from_op
        .reader(&from_path)
        .await?
        .into_futures_async_read(0..expected_size)
        .await?;
    let mut writer = to_op.writer(&to_path).await?;

    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut copied = 0_u64;
    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(read) => read,
            Err(error) => {
                if writer.abort().await.is_err() {
                    return Err(CoreError::TransferCleanupRequired);
                }
                return Err(error.into());
            }
        };
        if read == 0 {
            break;
        }
        copied = copied.saturating_add(read as u64);
        if copied > expected_size {
            if writer.abort().await.is_err() {
                return Err(CoreError::TransferCleanupRequired);
            }
            return Err(CoreError::Config(
                "Agent Task source changed while being copied".to_string(),
            ));
        }
        digest.update(&buffer[..read]);
        if let Err(error) = writer.write(buffer[..read].to_vec()).await {
            if writer.abort().await.is_err() {
                return Err(CoreError::TransferCleanupRequired);
            }
            return Err(error.into());
        }
    }

    let after = match from_op.stat(&from_path).await {
        Ok(after) => after,
        Err(error) => {
            if writer.abort().await.is_err() {
                return Err(CoreError::TransferCleanupRequired);
            }
            return Err(error.into());
        }
    };
    if after.is_dir() || after.content_length() != expected_size || copied != expected_size {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(CoreError::Config(
            "Agent Task source changed while being copied".to_string(),
        ));
    }

    if let Err(error) = writer.close().await {
        if writer.abort().await.is_err() {
            return Err(CoreError::TransferCleanupRequired);
        }
        return Err(error.into());
    }

    Ok(AgentTaskFileDigest {
        byte_size: copied,
        sha256: format!("{:x}", digest.finalize()),
    })
}

async fn ensure_planned_source(entry: &TransferPlanEntry, op: &Operator) -> Result<()> {
    let source_path = if entry.is_dir {
        format!("{}/", entry.source_path.trim_end_matches('/'))
    } else {
        entry.source_path.clone()
    };
    let metadata = op.stat(&source_path).await?;
    if metadata.is_dir() != entry.is_dir
        || (!entry.is_dir && metadata.content_length() != entry.size)
    {
        return Err(CoreError::Config(
            "Agent Task source changed after planning".to_string(),
        ));
    }
    Ok(())
}

async fn require_missing_destination(op: &Operator, path: &str) -> Result<()> {
    match op.stat(path).await {
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(CoreError::Config(
            "Agent Task destination changed after planning".to_string(),
        )),
    }
}

fn normalize_file_path(path: &str) -> Result<String> {
    let normalized = path.trim().trim_start_matches('/');
    if normalized.is_empty() || normalized.contains('\\') {
        return Err(CoreError::Config(
            "Agent Task file path must be a non-empty forward-slash path".to_string(),
        ));
    }
    if normalized
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(CoreError::Config(
            "Agent Task file path contains an invalid segment".to_string(),
        ));
    }
    Ok(normalized.to_string())
}

fn path_depth(path: &str) -> usize {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{plan_transfer_entries, TransferConflictPolicy, TransferOperation};

    fn memory_operator() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    #[tokio::test]
    async fn hashes_without_loading_through_task_manifest_api() {
        let op = memory_operator();
        op.write("inputs/report.txt", "hello agent task")
            .await
            .unwrap();

        let digest = hash_agent_task_file(&op, "inputs/report.txt")
            .await
            .unwrap();
        assert_eq!(digest.byte_size, 16);
        assert_eq!(
            digest.sha256,
            "28236dc73a012bb872284c9576dda020a85bef2041bbe976e45a8bf1c668d275"
        );
    }

    #[tokio::test]
    async fn rejects_directories_and_unsafe_paths() {
        let op = memory_operator();
        op.create_dir("inputs/").await.unwrap();
        assert!(hash_agent_task_file(&op, "inputs/").await.is_err());
        assert!(hash_agent_task_file(&op, "").await.is_err());
        assert!(normalize_file_path("inputs/../secret").is_err());
    }

    #[tokio::test]
    async fn exact_plan_does_not_copy_files_added_after_planning() {
        let source = memory_operator();
        let destination = memory_operator();
        source.create_dir("selected/").await.unwrap();
        source.write("selected/a.txt", "planned").await.unwrap();
        destination.create_dir("task/inputs/").await.unwrap();

        let plan = plan_transfer_entries(
            &source,
            &destination,
            vec!["selected".to_string()],
            "task/inputs",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        source.write("selected/b.txt", "unplanned").await.unwrap();
        copy_agent_task_plan(&source, &destination, &plan)
            .await
            .unwrap();

        assert_eq!(
            destination
                .read("task/inputs/selected/a.txt")
                .await
                .unwrap()
                .to_vec(),
            b"planned"
        );
        assert!(destination
            .stat("task/inputs/selected/b.txt")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn exact_plan_rejects_planned_file_size_drift() {
        let source = memory_operator();
        let destination = memory_operator();
        source.write("a.txt", "old").await.unwrap();
        destination.create_dir("task/inputs/").await.unwrap();

        let plan = plan_transfer_entries(
            &source,
            &destination,
            vec!["a.txt".to_string()],
            "task/inputs",
            TransferOperation::Copy,
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap();

        source.write("a.txt", "new-longer").await.unwrap();
        let error = copy_agent_task_plan(&source, &destination, &plan)
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::Config(_)));
        assert!(destination.stat("task/inputs/a.txt").await.is_err());
    }

    #[tokio::test]
    async fn copy_rejects_expected_size_mismatch_before_destination_write() {
        let source = memory_operator();
        let destination = memory_operator();
        source.write("a.txt", "larger").await.unwrap();

        let error = copy_agent_task_file(&source, "a.txt", &destination, "out/a.txt", 1)
            .await
            .unwrap_err();
        assert!(matches!(error, CoreError::Config(_)));
        assert!(destination.stat("out/a.txt").await.is_err());
    }
}
