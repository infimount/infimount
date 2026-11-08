//! Review-bound output selection for the offline remote-sandbox pilot.
//!
//! The remote candidate listing is not authority to choose files for import.
//! Require its exact canonical paths to equal the separately reviewed export
//! brief before any output destination write. This experimental entry point
//! still only supports isolated OpenDAL Memory on both sides.
use infimount_core::agent_tasks::{AgentTaskBrief, AgentTaskManifest};
use infimount_core::CoreError;
use opendal::Operator;

use crate::sandbox_bundle::build_sandbox_task_bundle;
use crate::sandbox_import::{
    validate_sandbox_output_listing, SandboxOutputCandidatePlan, SandboxRemoteOutputListing,
};
use crate::sandbox_memory_ingest::{
    ingest_sandbox_output_into_memory, InMemorySandboxIngestResult,
};

/// Purely verify the reviewed file set. This is not permission to publish.
/// Call with the trusted prepared manifest and separately approved export brief.
pub fn verify_reviewed_sandbox_output_scope(
    trusted_manifest: &AgentTaskManifest,
    reviewed_export_brief: &AgentTaskBrief,
    untrusted_candidate: &SandboxOutputCandidatePlan,
) -> Result<(), CoreError> {
    let bundle = build_sandbox_task_bundle(trusted_manifest, reviewed_export_brief)?;
    let rebuilt = validate_sandbox_output_listing(&SandboxRemoteOutputListing {
        schema_version: untrusted_candidate.schema_version,
        entries: untrusted_candidate.entries.clone(),
    })?;
    if rebuilt != *untrusted_candidate {
        return Err(CoreError::Config(
            "remote output listing changed since canonical validation".into(),
        ));
    }
    if rebuilt.entries.len() != bundle.requested_outputs.len()
        || !rebuilt
            .entries
            .iter()
            .map(|item| item.path.as_str())
            .eq(bundle.requested_outputs.iter().map(String::as_str))
    {
        return Err(CoreError::Config(
            "remote output paths differ from the separately reviewed deliverables".into(),
        ));
    }
    Ok(())
}

/// The offline, fully review-bound in-memory output integration path.
/// Recheck expected paths immediately before the importer independently
/// hashes every staged byte and writes create-only into task outputs.
///
/// This never writes a real filesystem or cloud destination, and it never
/// approves publication into original storage.
pub async fn ingest_reviewed_sandbox_outputs_into_memory(
    trusted_manifest: &AgentTaskManifest,
    reviewed_export_brief: &AgentTaskBrief,
    untrusted_candidate: &SandboxOutputCandidatePlan,
    staged: &Operator,
    destination_workspace: &Operator,
) -> Result<InMemorySandboxIngestResult, CoreError> {
    verify_reviewed_sandbox_output_scope(
        trusted_manifest,
        reviewed_export_brief,
        untrusted_candidate,
    )?;
    ingest_sandbox_output_into_memory(
        untrusted_candidate,
        staged,
        destination_workspace,
        &trusted_manifest.task_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_import::{
        SandboxRemoteEntryKind, SandboxRemoteOutputEntry, SANDBOX_IMPORT_SCHEMA_VERSION,
    };
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
    use sha2::{Digest, Sha256};

    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn memory() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    fn manifest() -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: TASK_ID.into(),
            title: "PRIVATE: original task title".into(),
            created_at: "2026-11-08T12:00:00Z".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_root: format!("tasks/{TASK_ID}"),
            outputs_directory: "outputs".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/sample.csv".into(),
                byte_size: 5,
                sha256: format!("{:x}", Sha256::digest(b"alpha")),
            }],
        }
    }

    fn brief() -> AgentTaskBrief {
        AgentTaskBrief {
            title: "Summarize reviewed data".into(),
            objective: "Produce only the two reviewed files.".into(),
            requested_outputs: vec!["outputs/report.md".into(), "outputs/data.csv".into()],
        }
    }

    fn file(path: &str, bytes: &[u8]) -> SandboxRemoteOutputEntry {
        SandboxRemoteOutputEntry {
            path: path.into(),
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
    async fn only_exact_reviewed_files_are_imported_create_only_into_memory() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/report.md", "summary").await.unwrap();
        staged.write("outputs/data.csv", "a,b").await.unwrap();
        let plan = candidate(vec![
            file("outputs/report.md", b"summary"),
            file("outputs/data.csv", b"a,b"),
        ]);
        let result = ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &plan,
            &staged,
            &dest,
        )
        .await
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.total_bytes, 10);
        assert_eq!(
            dest.read(&format!("tasks/{TASK_ID}/outputs/report.md"))
                .await
                .unwrap()
                .to_vec(),
            b"summary"
        );
        assert_eq!(
            dest.read(&format!("tasks/{TASK_ID}/outputs/data.csv"))
                .await
                .unwrap()
                .to_vec(),
            b"a,b"
        );
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &plan,
            &staged,
            &dest,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn unexpected_remote_file_is_denied_even_when_listing_and_bytes_agree() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/report.md", "summary").await.unwrap();
        staged.write("outputs/data.csv", "a,b").await.unwrap();
        staged
            .write("outputs/unrequested.txt", "private")
            .await
            .unwrap();
        let plan = candidate(vec![
            file("outputs/report.md", b"summary"),
            file("outputs/data.csv", b"a,b"),
            file("outputs/unrequested.txt", b"private"),
        ]);
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &plan,
            &staged,
            &dest,
        )
        .await
        .is_err());
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/report.md"))
            .await
            .is_err());
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/data.csv"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn missing_expected_output_fails_before_any_write() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/report.md", "summary").await.unwrap();
        let plan = candidate(vec![file("outputs/report.md", b"summary")]);
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &plan,
            &staged,
            &dest,
        )
        .await
        .is_err());
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/report.md"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn changed_review_or_forged_listing_never_changes_output_authority() {
        let mut reviewed = brief();
        let original = candidate(vec![
            file("outputs/report.md", b"summary"),
            file("outputs/data.csv", b"a,b"),
        ]);
        verify_reviewed_sandbox_output_scope(&manifest(), &reviewed, &original).unwrap();
        reviewed.requested_outputs = vec!["outputs/alternate.md".into()];
        assert!(verify_reviewed_sandbox_output_scope(&manifest(), &reviewed, &original).is_err());
        let mut forged = original.clone();
        forged.listing_metadata_sha256 = "b".repeat(64);
        assert!(verify_reviewed_sandbox_output_scope(&manifest(), &brief(), &forged).is_err());
        let mut invalid_task = manifest();
        invalid_task.task_id = "../escape".into();
        assert!(verify_reviewed_sandbox_output_scope(&invalid_task, &brief(), &original).is_err());
    }

    #[tokio::test]
    async fn matching_names_do_not_override_real_byte_verification() {
        let staged = memory();
        let dest = memory();
        staged.write("outputs/report.md", "changed").await.unwrap();
        staged.write("outputs/data.csv", "a,b").await.unwrap();
        let plan = candidate(vec![
            file("outputs/report.md", b"summary"),
            file("outputs/data.csv", b"a,b"),
        ]);
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &plan,
            &staged,
            &dest,
        )
        .await
        .is_err());
        assert!(dest
            .stat(&format!("tasks/{TASK_ID}/outputs/report.md"))
            .await
            .is_err());
    }
}
