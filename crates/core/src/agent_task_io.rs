use futures::io::AsyncReadExt;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{CoreError, Result};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskFileDigest {
    pub byte_size: u64,
    pub sha256: String,
}

/// Hash the exact bytes currently stored at `path` without loading the file
/// into memory. Size is checked before and after the read so a concurrent
/// truncate/extend is rejected. The digest is provenance only; publication
/// snapshots the reviewed bytes before any destination write.
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

/// Copy one file between OpenDAL operators while hashing the bytes actually
/// copied. The source is size-checked before/after the stream. When
/// `if_not_exists` is true, the destination backend must advertise atomic
/// create-if-absent support and the writer is opened with that condition.
///
/// Safe publication uses this twice: first to snapshot a reviewed task output
/// into app-owned local staging, then to upload that immutable snapshot with
/// atomic no-overwrite semantics.
pub async fn copy_agent_task_file(
    from_op: &Operator,
    from_path: &str,
    to_op: &Operator,
    to_path: &str,
    if_not_exists: bool,
) -> Result<AgentTaskFileDigest> {
    let from_path = normalize_file_path(from_path)?;
    let to_path = normalize_file_path(to_path)?;
    let before = from_op.stat(&from_path).await?;
    if before.is_dir() {
        return Err(CoreError::Config(
            "Agent Task copy source must refer to a file".to_string(),
        ));
    }
    let expected_size = before.content_length();
    if if_not_exists && !to_op.info().capability().write_with_if_not_exists {
        return Err(CoreError::Config(
            "destination backend cannot guarantee an atomic no-overwrite write".to_string(),
        ));
    }

    let mut reader = from_op
        .reader(&from_path)
        .await?
        .into_futures_async_read(0..expected_size)
        .await?;
    let mut writer = if if_not_exists {
        to_op.writer_with(&to_path).if_not_exists(true).await?
    } else {
        to_op.writer(&to_path).await?
    };

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

    let after = from_op.stat(&from_path).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn copy_hashes_the_bytes_that_were_copied() {
        let source = memory_operator();
        let destination = memory_operator();
        source.write("out/report.txt", "approved bytes").await.unwrap();

        let digest = copy_agent_task_file(
            &source,
            "out/report.txt",
            &destination,
            "published/report.txt",
            false,
        )
        .await
        .unwrap();
        assert_eq!(digest.byte_size, 14);
        assert_eq!(
            digest.sha256,
            "5f7730eeb4cbfa3d0e84c8c98324072c1dc903c0b6c4d85e143389c86e9872f7"
        );
        assert_eq!(
            destination
                .read("published/report.txt")
                .await
                .unwrap()
                .to_vec(),
            b"approved bytes"
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
}
