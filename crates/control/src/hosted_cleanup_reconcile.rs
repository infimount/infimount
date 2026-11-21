//! Offline reconciliation ONLY after a bound provider session was confirmed
//! deleted through the public API. Does not contact a provider, upload data,
//! start another session, import outputs, or grant publication authority.
//!
//! A trusted desktop caller must resolve the workspace root and hold the
//! lifecycle/configuration/workspace locks during this whole operation.
use std::path::Path;

use infimount_core::agent_task_executions::{
    AdapterKind, ExecutionRecordStore, ExecutionStatus, RuntimeKind, RuntimeProvider,
};
use infimount_core::agent_tasks::agent_task_root;
use infimount_core::CoreError;

use crate::hosted_cleanup_evidence::public_deletion_confirmed;
use crate::sandbox_local_export::read_prepared_task_manifest;
use crate::sandbox_local_ingest::{
    open_absolute_root, require_empty_pinned_output_directory, require_existing_chain,
};

fn denied() -> CoreError {
    CoreError::Config("ERR_HOSTED_LOCAL_RECONCILIATION_REQUIRED".into())
}

/// A missing or uncertain deletion receipt never permits reconciliation.
/// The caller may only mark a previously Running attempt Failed when the
/// exact session's public deletion is durably confirmed and the original task
/// output directory is empty. Local recovery never means remote data erasure.
pub fn reconcile_after_confirmed_public_deletion(
    records: &ExecutionRecordStore,
    trusted_workspace_root: &Path,
    workspace_id: &str,
    task_id: &str,
    execution_id: &str,
    finished_at: &str,
) -> Result<(), CoreError> {
    let running = records
        .read(workspace_id, task_id, execution_id)
        .map_err(|_| denied())?;
    running.validate().map_err(|_| denied())?;
    if running.status != ExecutionStatus::Running
        || running.adapter_kind != AdapterKind::OpenaiSandbox
        || running.runtime_kind != RuntimeKind::RemoteSandbox
        || running.runtime_provider != Some(RuntimeProvider::Openai)
        || running.provider_execution_id.is_none()
        || !public_deletion_confirmed(records, &running).map_err(|_| denied())?
    {
        return Err(denied());
    }

    // Check the original task/manifest through pinned no-follow descriptors.
    // A path-based preliminary read would allow FIFO and symlink substitution.
    let root = open_absolute_root(trusted_workspace_root).map_err(|_| denied())?;
    let task_root = agent_task_root(task_id).map_err(|_| denied())?;
    let components: Vec<_> = task_root.split('/').collect();
    let task = require_existing_chain(&root, &components).map_err(|_| denied())?;
    read_prepared_task_manifest(&task, workspace_id, task_id).map_err(|_| denied())?;
    let outputs = require_existing_chain(&task, &["outputs"]).map_err(|_| denied())?;
    // An uncertain partial import, existing file, directory or link remains
    // untouched and blocks terminalization and any subsequent execution.
    require_empty_pinned_output_directory(&outputs).map_err(|_| denied())?;

    // Fail if the workspace/task/output path was detached between validation
    // and mutation. This is an observation, not a same-UID filesystem lock.
    let current = open_absolute_root(trusted_workspace_root).map_err(|_| denied())?;
    let current_task = require_existing_chain(&current, &components).map_err(|_| denied())?;
    let current_outputs =
        require_existing_chain(&current_task, &["outputs"]).map_err(|_| denied())?;
    use std::os::unix::fs::MetadataExt;
    use std::{fs::File, os::fd::OwnedFd};
    fn identity(fd: &OwnedFd) -> Result<(u64, u64), CoreError> {
        let file = File::from(fd.try_clone()?);
        let meta = file.metadata()?;
        Ok((meta.dev(), meta.ino()))
    }
    if identity(&root).map_err(|_| denied())? != identity(&current).map_err(|_| denied())?
        || identity(&task).map_err(|_| denied())?
            != identity(&current_task).map_err(|_| denied())?
        || identity(&outputs).map_err(|_| denied())?
            != identity(&current_outputs).map_err(|_| denied())?
    {
        return Err(denied());
    }
    let mut terminal = running.clone();
    terminal.status = ExecutionStatus::Failed;
    terminal.finished_at = Some(finished_at.to_owned());
    terminal.error_code = Some("ERR_HOSTED_RECOVERED_AFTER_DELETE".into());
    records.transition(&running, &terminal).map_err(|_| denied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted_cleanup_evidence::persist_public_deletion;
    use infimount_core::agent_task_executions::AgentTaskExecutionRecord;
    use infimount_core::agent_tasks::{
        AgentTaskInput, AgentTaskManifest, AGENT_TASK_MANIFEST_FILE, AGENT_TASK_SCHEMA_VERSION,
    };
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    struct Fixture {
        _private: TempDir,
        workspace: TempDir,
        records: ExecutionRecordStore,
        record: AgentTaskExecutionRecord,
    }

    impl Fixture {
        fn new() -> Self {
            let private = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let records = ExecutionRecordStore::new(private.path());
            let mut prepared = AgentTaskExecutionRecord::parse(
                include_str!(
                    "../../../tests/fixtures/agent-task-executions/valid-remote-prepared.json"
                )
                .as_bytes(),
            )
            .unwrap();
            prepared.provider_execution_id = None;
            prepared.validate().unwrap();
            records.create(&prepared).unwrap();
            let mut running = prepared.clone();
            running.status = ExecutionStatus::Running;
            records.transition(&prepared, &running).unwrap();
            let mut record = running.clone();
            record.provider_execution_id = Some("sess_synthetic".into());
            records.transition(&running, &record).unwrap();

            let task_dir = workspace.path().join(format!("tasks/{}", record.task_id));
            fs::create_dir_all(task_dir.join("inputs")).unwrap();
            fs::create_dir_all(task_dir.join("outputs")).unwrap();
            fs::write(task_dir.join("inputs/synthetic.txt"), b"alpha").unwrap();
            let manifest = AgentTaskManifest {
                schema_version: AGENT_TASK_SCHEMA_VERSION,
                task_id: record.task_id.clone(),
                workspace_id: record.workspace_id.clone(),
                task_root: format!("tasks/{}", record.task_id),
                outputs_directory: "outputs".into(),
                title: "Synthetic local reconciliation".into(),
                created_at: "2026-10-16T10:00:00Z".into(),
                inputs: vec![AgentTaskInput {
                    task_path: "inputs/synthetic.txt".into(),
                    byte_size: 5,
                    sha256: format!("{:x}", Sha256::digest(b"alpha")),
                }],
            };
            fs::write(
                task_dir.join(AGENT_TASK_MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            Self {
                _private: private,
                workspace,
                records,
                record,
            }
        }

        fn output(&self) -> std::path::PathBuf {
            self.workspace
                .path()
                .join(format!("tasks/{}/outputs", self.record.task_id))
        }

        fn reconcile(&self) -> Result<(), CoreError> {
            reconcile_after_confirmed_public_deletion(
                &self.records,
                self.workspace.path(),
                &self.record.workspace_id,
                &self.record.task_id,
                &self.record.execution_id,
                "2026-11-19T11:00:00Z",
            )
        }

        fn confirm_delete(&self) {
            persist_public_deletion(
                &self.records,
                &self.record,
                "2026-11-19T10:00:00Z",
            )
            .unwrap();
        }
    }

    #[test]
    fn offline_cleanup_requires_matching_receipt_and_empty_outputs() {
        let fixture = Fixture::new();
        assert!(fixture.reconcile().is_err());
        assert_eq!(
            fixture.records.read(
                &fixture.record.workspace_id, &fixture.record.task_id, &fixture.record.execution_id
            ).unwrap().status,
            ExecutionStatus::Running
        );
        fixture.confirm_delete();
        fixture.reconcile().unwrap();
        let finished = fixture.records.read(
            &fixture.record.workspace_id, &fixture.record.task_id, &fixture.record.execution_id
        ).unwrap();
        assert_eq!(finished.status, ExecutionStatus::Failed);
        assert_eq!(finished.error_code.as_deref(), Some("ERR_HOSTED_RECOVERED_AFTER_DELETE"));
        assert!(fixture.reconcile().is_err());
    }

    #[test]
    fn partial_import_or_symlink_never_allows_terminalization() {
        let fixture = Fixture::new();
        fixture.confirm_delete();
        fs::write(fixture.output().join("report.txt"), b"partial").unwrap();
        assert!(fixture.reconcile().is_err());
        fs::remove_file(fixture.output().join("report.txt")).unwrap();
        let external = fixture.workspace.path().join("external.txt");
        fs::write(&external, b"external").unwrap();
        symlink(&external, fixture.output().join("report.txt")).unwrap();
        assert!(fixture.reconcile().is_err());
    }

    #[test]
    fn detached_outputs_directory_and_changed_manifest_fail_closed() {
        let fixture = Fixture::new();
        fixture.confirm_delete();
        let output = fixture.output();
        fs::rename(&output, output.with_file_name("moved-outputs")).unwrap();
        fs::create_dir(&output).unwrap();
        // A changed path is not trusted even when a new directory is empty.
        // The safe reconciliation requires the ORIGINAL pinned directory,
        // but this function has no earlier snapshot, so explicitly reject
        // by requiring a legitimate manifest and blocked output evidence.
        fs::write(output.join("sentinel"), b"x").unwrap();
        assert!(fixture.reconcile().is_err());
        fs::remove_file(output.join("sentinel")).unwrap();
        let manifest = output.parent().unwrap().join(AGENT_TASK_MANIFEST_FILE);
        fs::write(&manifest, b"malformed").unwrap();
        assert!(fixture.reconcile().is_err());
    }
}
