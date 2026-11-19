//! Private, create-only evidence that an OpenAI session DELETE returned a
//! matching public-API deletion confirmation. It does NOT assert physical
//! erasure or provider retention expiry. No session ID crosses desktop IPC.
//!
//! Separate from immutable v1 execution records. This preserves old history
//! and prevents an undocumented change to the 21-field record contract.
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use infimount_core::agent_task_executions::{
    AdapterKind, AgentTaskExecutionRecord, ExecutionRecordStore, ExecutionStatus, RuntimeKind,
    RuntimeProvider,
};
use infimount_core::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sandbox_local_ingest::open_absolute_root;

const RECEIPTS_DIR: &str = "hosted-public-deletion-v1";
const MAX_RECEIPT_BYTES: u64 = 2048;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicDeletionReceipt {
    schema_version: u32,
    workspace_id: String,
    task_id: String,
    execution_id: String,
    provider_session_sha256: String,
    confirmed_at: String,
    public_api_deleted: bool,
    physical_deletion_confirmed: bool,
}

fn rejected() -> CoreError {
    CoreError::Config("ERR_HOSTED_DELETION_RECEIPT".into())
}

fn checked_hosted_record(record: &AgentTaskExecutionRecord) -> Result<&str, CoreError> {
    record.validate()?;
    if record.adapter_kind != AdapterKind::OpenaiSandbox
        || record.runtime_kind != RuntimeKind::RemoteSandbox
        || record.runtime_provider != Some(RuntimeProvider::Openai)
    {
        return Err(rejected());
    }
    record.provider_execution_id.as_deref().ok_or_else(rejected)
}

fn name(value: &str) -> Result<CString, CoreError> {
    CString::new(value).map_err(|_| rejected())
}

fn directory_at(parent: &OwnedFd, component: &str) -> io::Result<Option<OwnedFd>> {
    let component = CString::new(component).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: pinned directory descriptor, one NUL-terminated component.
    let fd = unsafe {
        libc::openat(parent.as_raw_fd(), component.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
    };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: openat returned an owned descriptor.
    Ok(Some(unsafe { OwnedFd::from_raw_fd(fd) }))
}

fn require_or_create(parent: &OwnedFd, component: &str) -> Result<OwnedFd, CoreError> {
    let component_c = name(component)?;
    // SAFETY: one validated component beneath a pinned parent.
    let rc = unsafe { libc::mkdirat(parent.as_raw_fd(), component_c.as_ptr(), 0o700) };
    if rc < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(rejected());
        }
    }
    let child = directory_at(parent, component)
        .map_err(|_| rejected())?
        .ok_or_else(rejected)?;
    if rc == 0 {
        File::from(child.try_clone()?).sync_all()?;
        File::from(parent.try_clone()?).sync_all()?;
    }
    Ok(child)
}

fn starting_directory(store: &ExecutionRecordStore) -> Result<OwnedFd, CoreError> {
    let root = store.root().parent().ok_or_else(rejected)?;
    open_absolute_root(root).map_err(|_| rejected())
}

fn verified_receipt(
    receipt: &PublicDeletionReceipt,
    record: &AgentTaskExecutionRecord,
    session_id: &str,
) -> Result<(), CoreError> {
    if receipt.schema_version != 1
        || receipt.workspace_id != record.workspace_id
        || receipt.task_id != record.task_id
        || receipt.execution_id != record.execution_id
        || receipt.provider_session_sha256 != format!("{:x}", Sha256::digest(session_id))
        || !receipt.public_api_deleted
        || receipt.physical_deletion_confirmed
        || receipt.confirmed_at.len() > 48
        || receipt.confirmed_at.is_empty()
        || !receipt.confirmed_at.bytes().all(|b| b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b':' | b'.' | b'+' ))
    {
        return Err(rejected());
    }
    Ok(())
}

/// Invoked ONLY after a verified delete-session response, before importing a
/// single byte. Any failed sync leaves the run Running and requires review.
pub(crate) fn persist_public_deletion(
    store: &ExecutionRecordStore,
    running: &AgentTaskExecutionRecord,
    confirmed_at: &str,
) -> Result<(), CoreError> {
    let session_id = checked_hosted_record(running)?;
    if running.status != ExecutionStatus::Running {
        return Err(rejected());
    }
    let receipt = PublicDeletionReceipt {
        schema_version: 1,
        workspace_id: running.workspace_id.clone(),
        task_id: running.task_id.clone(),
        execution_id: running.execution_id.clone(),
        provider_session_sha256: format!("{:x}", Sha256::digest(session_id)),
        confirmed_at: confirmed_at.to_owned(),
        public_api_deleted: true,
        physical_deletion_confirmed: false,
    };
    verified_receipt(&receipt, running, session_id)?;
    let bytes = serde_json::to_vec(&receipt)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(rejected());
    }
    let config = starting_directory(store)?;
    let root = require_or_create(&config, RECEIPTS_DIR)?;
    let workspace = require_or_create(&root, &running.workspace_id)?;
    let task = require_or_create(&workspace, &running.task_id)?;
    let leaf = name(&format!("{}.json", running.execution_id))?;
    // SAFETY: create-new, no-follow file under pinned directory.
    let fd = unsafe {
        libc::openat(task.as_raw_fd(), leaf.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600)
    };
    if fd < 0 {
        return Err(rejected());
    }
    // SAFETY: successful openat gave one owned file.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::from(task.try_clone()?).sync_all()?;
    Ok(())
}

/// Public read-only projection. A malformed or linked receipt fails closed.
/// Missing evidence is NOT a claim that deletion never happened.
pub fn public_deletion_confirmed(
    store: &ExecutionRecordStore,
    record: &AgentTaskExecutionRecord,
) -> Result<bool, CoreError> {
    let session_id = match checked_hosted_record(record) {
        Ok(id) => id,
        Err(_) if record.provider_execution_id.is_none() => return Ok(false),
        Err(error) => return Err(error),
    };
    let config = starting_directory(store)?;
    let mut parent = config;
    for component in [RECEIPTS_DIR, &record.workspace_id, &record.task_id] {
        parent = match directory_at(&parent, component).map_err(|_| rejected())? {
            Some(dir) => dir,
            None => return Ok(false),
        };
    }
    let leaf = name(&format!("{}.json", record.execution_id))?;
    // SAFETY: confined, non-blocking, no-follow read of a fixed file name.
    let fd = unsafe {
        libc::openat(parent.as_raw_fd(), leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
    };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(rejected())
        };
    }
    // SAFETY: successful openat gave one owned file.
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_RECEIPT_BYTES
    {
        return Err(rejected());
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(rejected());
    }
    let receipt: PublicDeletionReceipt = serde_json::from_slice(&bytes).map_err(|_| rejected())?;
    verified_receipt(&receipt, record, session_id)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_task_executions::{AgentTaskExecutionRecord, ExecutionStatus};
    use std::fs;
    use std::os::unix::fs::symlink;

    fn fixture() -> AgentTaskExecutionRecord {
        let mut record = AgentTaskExecutionRecord::parse(
            include_str!("../../../tests/fixtures/agent-task-executions/valid-remote-prepared.json").as_bytes()
        ).unwrap();
        record.status = ExecutionStatus::Running;
        record.provider_execution_id = Some("sess_synthetic".into());
        record.validate().unwrap();
        record
    }

    #[test]
    fn receipt_is_create_only_bound_and_readable_after_terminal_transition() {
        let home = tempfile::tempdir().unwrap();
        let store = ExecutionRecordStore::new(home.path());
        let running = fixture();
        assert!(!public_deletion_confirmed(&store, &running).unwrap());
        persist_public_deletion(&store, &running, "2026-11-17T07:00:00Z").unwrap();
        assert!(public_deletion_confirmed(&store, &running).unwrap());
        assert!(persist_public_deletion(&store, &running, "2026-11-17T07:00:01Z").is_err());
        let mut finished = running.clone();
        finished.status = ExecutionStatus::Failed;
        finished.finished_at = Some("2026-11-17T07:01:00Z".into());
        finished.error_code = Some("ERR_PROVIDER_FAILED".into());
        assert!(public_deletion_confirmed(&store, &finished).unwrap());
        finished.provider_execution_id = Some("sess_swapped".into());
        assert!(public_deletion_confirmed(&store, &finished).is_err());
    }

    #[test]
    fn linked_or_corrupted_receipts_never_prove_deletion() {
        let home = tempfile::tempdir().unwrap();
        let store = ExecutionRecordStore::new(home.path());
        let running = fixture();
        persist_public_deletion(&store, &running, "2026-11-17T07:00:00Z").unwrap();
        let file = home.path().join(RECEIPTS_DIR)
            .join(&running.workspace_id).join(&running.task_id)
            .join(format!("{}.json", running.execution_id));
        let original = fs::read(&file).unwrap();
        fs::write(&file, b"not json").unwrap();
        assert!(public_deletion_confirmed(&store, &running).is_err());
        fs::remove_file(&file).unwrap();
        let linked_target = home.path().join("external");
        fs::write(&linked_target, original).unwrap();
        symlink(&linked_target, &file).unwrap();
        assert!(public_deletion_confirmed(&store, &running).is_err());
    }
}
