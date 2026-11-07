//! Bounded C1.3a sandbox output verification in isolated memory storage.
//!
//! This experimental helper is deliberately restricted to OpenDAL Memory on
//! BOTH sides. It cannot write local files, cloud storage, original source
//! systems, or a production Agent Workspace. A future trusted importer must
//! qualify file-descriptor confinement, no-follow filesystem traversal,
//! workspace locks and private execution evidence before allowing FS output.
//!
//! The provider's declared output listing is untrusted. Verify the ENTIRE
//! in-memory tree, hash every byte before writing, then use the same create-only
//! writer and integrity checks as desktop-reviewed publication. A metadata
//! digest and a supplied file type are never authorization to publish.
use infimount_core::agent_task_io::{
    hash_agent_task_file, publish_reviewed_agent_task_file, AgentTaskFileDigest,
};
use infimount_core::agent_tasks::agent_task_root;
use infimount_core::CoreError;
use opendal::{ErrorKind, Operator};
use serde::Serialize;

use crate::review_evidence::list_review_output_entries;
use crate::sandbox_import::{
    validate_sandbox_output_listing, SandboxOutputCandidatePlan, SandboxRemoteOutputEntry,
    SandboxRemoteOutputListing,
};

/// Evidence of a completed transfer between two isolated in-memory stores,
/// NOT a user-approved publication receipt or a native presence grant.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InMemorySandboxIngestResult {
    pub task_id: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<SandboxRemoteOutputEntry>,
}

/// Exercise the full create-only verified byte loop on in-memory staging.
///
/// The method rejects different OpenDAL backends instead of trying to treat
/// a symlink-following host filesystem as trusted. The future real importer
/// must be a separate reviewed service with no-follow handles and workspace
/// mutation locks. No actor/MCP caller receives publication rights here.
pub async fn ingest_sandbox_output_into_memory(
    candidate: &SandboxOutputCandidatePlan,
    staged: &Operator,
    destination_workspace: &Operator,
    trusted_task_id: &str,
) -> Result<InMemorySandboxIngestResult, CoreError> {
    if staged.info().scheme() != "memory" || destination_workspace.info().scheme() != "memory" {
        return Err(CoreError::Config(
            "sandbox ingestion prototype requires isolated memory storage".into(),
        ));
    }

    // Public fields on an untrusted candidate must not allow callers to
    // replace the path list or digest after a separate validation step.
    let rebuilt = validate_sandbox_output_listing(&SandboxRemoteOutputListing {
        schema_version: candidate.schema_version,
        entries: candidate.entries.clone(),
    })?;
    if &rebuilt != candidate {
        return Err(CoreError::Config(
            "sandbox output candidate changed since validation".into(),
        ));
    }

    let task_root = agent_task_root(trusted_task_id)?;
    let staged_files = list_review_output_entries(staged, "outputs").await?;
    if staged_files.len() != candidate.entries.len() {
        return Err(CoreError::Config(
            "sandbox staged file inventory differs from reviewed candidate".into(),
        ));
    }

    // Reject unexpected files and size drift BEFORE the first destination
    // writer is opened. Memory operator cannot contain filesystem symlinks.
    for (actual, declared) in staged_files.iter().zip(&candidate.entries) {
        if actual.path != declared.path || actual.size != declared.byte_size {
            return Err(CoreError::Config(
                "sandbox staged file inventory differs from reviewed candidate".into(),
            ));
        }
    }

    let mut destination_paths = Vec::with_capacity(candidate.entries.len());
    let mut measured_total = 0_u64;
    for file in &candidate.entries {
        let expected = AgentTaskFileDigest {
            byte_size: file.byte_size,
            sha256: file.sha256.clone(),
        };
        let measured = hash_agent_task_file(staged, &file.path).await?;
        if measured != expected {
            return Err(CoreError::Config(
                "sandbox staged bytes differ from declared candidate".into(),
            ));
        }
        measured_total = measured_total
            .checked_add(measured.byte_size)
            .ok_or_else(|| CoreError::Config("sandbox measured byte total overflowed".into()))?;
        let destination = format!("{task_root}/{}", file.path);
        // The unique destination is derived ONLY from trusted task ID plus
        // the validated sandbox-relative outputs/ path.
        match destination_workspace.stat(&destination).await {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => {
                return Err(CoreError::Config(
                    "sandbox destination output already exists".into(),
                ))
            }
        }
        destination_paths.push(destination);
    }
    if measured_total != candidate.total_bytes {
        return Err(CoreError::Config(
            "sandbox measured output bytes changed since validation".into(),
        ));
    }
    if !destination_workspace
        .info()
        .capability()
        .write_with_if_not_exists
    {
        return Err(CoreError::Config(
            "sandbox output destination lacks safe create-only writes".into(),
        ));
    }

    let planned = candidate.entries.iter().zip(&destination_paths);
    for (committed, (file, destination)) in planned.enumerate() {
        let expected = AgentTaskFileDigest {
            byte_size: file.byte_size,
            sha256: file.sha256.clone(),
        };
        // Verify source before/while copying and the committed destination
        // after close. The writer uses if-not-exists rather than overwrite.
        let result = publish_reviewed_agent_task_file(
            staged,
            &file.path,
            destination_workspace,
            destination,
            &expected,
        )
        .await;
        if let Err(error) = result {
            // Never delete potentially modified destination bytes. A
            // partial commit requires explicit user-visible cleanup.
            return Err(if committed == 0 {
                error
            } else {
                CoreError::TransferCleanupRequired
            });
        }
    }

    Ok(InMemorySandboxIngestResult {
        task_id: trusted_task_id.to_owned(),
        file_count: candidate.entries.len(),
        total_bytes: measured_total,
        files: candidate.entries.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_import::{SandboxRemoteEntryKind, SANDBOX_IMPORT_SCHEMA_VERSION};
    use sha2::{Digest, Sha256};

    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn memory() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    fn file(path: &str, bytes: &[u8]) -> SandboxRemoteOutputEntry {
        SandboxRemoteOutputEntry {
            path: path.to_owned(),
            kind: SandboxRemoteEntryKind::File,
            byte_size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    fn candidate(entries: Vec<SandboxRemoteOutputEntry>) -> SandboxOutputCandidatePlan {
        validate_sandbox_output_listing(&SandboxRemoteOutputListing {
            schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn isolated_ingest_verifies_real_bytes_and_creates_exact_task_outputs() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/z/report.md", "second").await.unwrap();
        staged.write("outputs/a.md", "first").await.unwrap();
        let plan = candidate(vec![
            file("outputs/z/report.md", b"second"),
            file("outputs/a.md", b"first"),
        ]);
        let result = ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID)
            .await
            .unwrap();
        assert_eq!(result.total_bytes, 11);
        assert_eq!(result.file_count, 2);
        assert_eq!(result.files[0].path, "outputs/a.md");
        assert_eq!(
            dest.read(&format!("tasks/{TASK_ID}/outputs/a.md"))
                .await
                .unwrap()
                .to_vec(),
            b"first"
        );
        assert_eq!(
            dest.read(&format!("tasks/{TASK_ID}/outputs/z/report.md"))
                .await
                .unwrap()
                .to_vec(),
            b"second"
        );
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn mismatched_same_size_bytes_reject_before_any_destination_write() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/summary.md", "evil").await.unwrap();
        let plan = candidate(vec![file("outputs/summary.md", b"good")]);
        let result = ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID).await;
        assert!(result.is_err());
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/summary.md"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn unlisted_extra_file_rejects_entire_candidate_before_commit() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/summary.md", "good").await.unwrap();
        staged.write("outputs/hidden.json", "{}").await.unwrap();
        let plan = candidate(vec![file("outputs/summary.md", b"good")]);
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID)
                .await
                .is_err()
        );
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/summary.md"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn preexisting_destination_is_never_replaced() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/result.txt", "new").await.unwrap();
        let dest_path = format!("tasks/{TASK_ID}/outputs/result.txt");
        dest.write(&dest_path, "sentinel").await.unwrap();
        let plan = candidate(vec![file("outputs/result.txt", b"new")]);
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID)
                .await
                .is_err()
        );
        assert_eq!(dest.read(&dest_path).await.unwrap().to_vec(), b"sentinel");
    }

    #[tokio::test]
    async fn forged_candidate_fingerprint_and_invalid_task_fail_closed() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/result.txt", "new").await.unwrap();
        let mut plan = candidate(vec![file("outputs/result.txt", b"new")]);
        plan.listing_metadata_sha256 = "a".repeat(64);
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &dest, TASK_ID)
                .await
                .is_err()
        );
        let plan = candidate(vec![file("outputs/result.txt", b"new")]);
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &dest, "../escape")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn non_memory_source_or_destination_is_rejected() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/ok", "a").await.unwrap();
        let plan = candidate(vec![file("outputs/ok", b"a")]);
        let temporary = tempfile::TempDir::new().unwrap();
        let fs = Operator::new(
            opendal::services::Fs::default().root(temporary.path().to_string_lossy().as_ref()),
        )
        .unwrap();
        assert!(
            ingest_sandbox_output_into_memory(&plan, &fs, &dest, TASK_ID)
                .await
                .is_err()
        );
        assert!(
            ingest_sandbox_output_into_memory(&plan, &staged, &fs, TASK_ID)
                .await
                .is_err()
        );
    }
}
