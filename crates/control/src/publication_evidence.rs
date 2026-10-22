//! Shared read-only evidence verification for an approved output selection.
//!
//! Callers must resolve the bound Agent Task workspace and validate the
//! resulting source path against that workspace's storage confinement before
//! invoking this helper. A verified digest is evidence, NOT publish authority.
use infimount_core::agent_task_io::{hash_agent_task_file, AgentTaskFileDigest};
use infimount_core::CoreError;
use opendal::Operator;

use crate::publication::{validate_output_task_path, validate_sha256, ReviewedAgentTaskOutput};

/// Verify the actual bytes on a trusted, task-confined path against the
/// selection reviewed by a human. Hash verification must be repeated during
/// publication under the authoritative mutation locks.
pub async fn verify_reviewed_output_digest(
    workspace_op: &Operator,
    confined_source_path: &str,
    reviewed: &ReviewedAgentTaskOutput,
) -> Result<AgentTaskFileDigest, CoreError> {
    validate_output_task_path(&reviewed.task_path)?;
    validate_sha256(&reviewed.sha256)?;
    // Never verify bytes from a different file as though they matched the
    // reviewed relative output. Namespace confinement remains the caller's job.
    let suffix = format!("/{}", reviewed.task_path);
    if confined_source_path != reviewed.task_path && !confined_source_path.ends_with(&suffix) {
        return Err(CoreError::Config(
            "reviewed output path does not match the confined source path".into(),
        ));
    }

    let metadata = workspace_op.stat(confined_source_path).await.map_err(|_| {
        CoreError::Config("A reviewed Agent Task output is no longer available".into())
    })?;
    if !metadata.is_file() || metadata.content_length() != reviewed.byte_size {
        return Err(CoreError::Config(
            "Agent Task output changed since it was reviewed".into(),
        ));
    }

    let current = hash_agent_task_file(workspace_op, confined_source_path).await?;
    if current.byte_size != reviewed.byte_size || current.sha256 != reviewed.sha256 {
        return Err(CoreError::Config(
            "Agent Task output changed since it was reviewed".into(),
        ));
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_operator() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    fn reviewed(digest: &AgentTaskFileDigest) -> ReviewedAgentTaskOutput {
        ReviewedAgentTaskOutput {
            task_path: "outputs/report.md".into(),
            byte_size: digest.byte_size,
            sha256: digest.sha256.clone(),
        }
    }

    #[tokio::test]
    async fn verified_digest_matches_review_and_rejects_same_size_drift() {
        let op = memory_operator();
        let path = "tasks/task-a/outputs/report.md";
        op.write(path, "version-1").await.unwrap();
        let original = hash_agent_task_file(&op, path).await.unwrap();
        let selection = reviewed(&original);

        assert_eq!(
            verify_reviewed_output_digest(&op, path, &selection)
                .await
                .unwrap(),
            original
        );

        op.write(path, "version-2").await.unwrap();
        assert!(verify_reviewed_output_digest(&op, path, &selection)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn rejects_missing_changed_size_and_non_file_outputs() {
        let op = memory_operator();
        let path = "tasks/task-a/outputs/report.md";
        op.write(path, "reviewed").await.unwrap();
        let original = hash_agent_task_file(&op, path).await.unwrap();
        let selection = reviewed(&original);

        assert!(verify_reviewed_output_digest(&op, "missing.md", &selection)
            .await
            .is_err());

        op.write(path, "reviewed and modified").await.unwrap();
        assert!(verify_reviewed_output_digest(&op, path, &selection)
            .await
            .is_err());

        op.create_dir("tasks/task-a/outputs/directory/")
            .await
            .unwrap();
        assert!(
            verify_reviewed_output_digest(&op, "tasks/task-a/outputs/directory/", &selection)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn identical_bytes_at_a_different_path_do_not_match_the_reviewed_selection() {
        let op = memory_operator();
        let reviewed_path = "tasks/task-a/outputs/report.md";
        let wrong_path = "tasks/task-a/outputs/another.md";
        op.write(reviewed_path, "reviewed").await.unwrap();
        op.write(wrong_path, "reviewed").await.unwrap();
        let digest = hash_agent_task_file(&op, reviewed_path).await.unwrap();

        assert!(
            verify_reviewed_output_digest(&op, wrong_path, &reviewed(&digest))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_unreviewed_paths_and_hash_encodings() {
        let op = memory_operator();
        let path = "tasks/task-a/outputs/report.md";
        op.write(path, "reviewed").await.unwrap();
        let original = hash_agent_task_file(&op, path).await.unwrap();
        let mut selection = reviewed(&original);

        selection.task_path = "inputs/private.txt".into();
        assert!(verify_reviewed_output_digest(&op, path, &selection)
            .await
            .is_err());

        selection.task_path = "outputs/report.md".into();
        selection.sha256 = original.sha256.to_uppercase();
        assert!(verify_reviewed_output_digest(&op, path, &selection)
            .await
            .is_err());
    }
}
