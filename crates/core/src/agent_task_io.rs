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
/// truncate/extend is rejected. The digest is provenance only; callers that
/// later approve publication must hash again immediately before writing.
pub async fn hash_agent_task_file(op: &Operator, path: &str) -> Result<AgentTaskFileDigest> {
    let normalized = path.trim().trim_start_matches('/');
    if normalized.is_empty() || normalized.contains('\\') {
        return Err(CoreError::Config(
            "Agent Task hash path must be a non-empty forward-slash path".to_string(),
        ));
    }

    let before = op.stat(normalized).await?;
    if before.is_dir() {
        return Err(CoreError::Config(
            "Agent Task hash path must refer to a file".to_string(),
        ));
    }
    let expected_size = before.content_length();
    let mut reader = op
        .reader(normalized)
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

    let after = op.stat(normalized).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_operator() -> Operator {
        Operator::new(opendal::services::Memory::default())
            .unwrap()
            .finish()
    }

    #[tokio::test]
    async fn hashes_without_loading_through_task_manifest_api() {
        let op = memory_operator();
        op.write("inputs/report.txt", "hello agent task")
            .await
            .unwrap();

        let digest = hash_agent_task_file(&op, "inputs/report.txt").await.unwrap();
        assert_eq!(digest.byte_size, 16);
        assert_eq!(
            digest.sha256,
            "28236dc73a012bb872284c9576dda020a85bef2041bbe976e45a8bf1c668d275"
        );
    }

    #[tokio::test]
    async fn rejects_directories_and_empty_paths() {
        let op = memory_operator();
        op.create_dir("inputs/").await.unwrap();
        assert!(hash_agent_task_file(&op, "inputs/").await.is_err());
        assert!(hash_agent_task_file(&op, "").await.is_err());
    }
}
