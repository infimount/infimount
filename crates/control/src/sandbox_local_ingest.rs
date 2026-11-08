//! Experimental Unix no-follow, create-only Agent Task output importer.
//!
//! This intentionally has NO Tauri command, MCP tool, or cloud API surface.
//! A future trusted caller must hold the workspace/configuration transaction
//! locks and supply C2-verified downloaded bytes. Original storage publication
//! still requires separately approved native human presence and fresh review.
//!
//! Every directory is opened relative to a pinned directory descriptor with
//! O_NOFOLLOW|O_DIRECTORY. Files are created using openat(O_EXCL|O_NOFOLLOW).
//! A failed write can leave partial bytes, which are NEVER silently deleted.
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use crate::sandbox_import::SandboxOutputCandidatePlan;
use crate::sandbox_reviewed_ingest::verify_reviewed_sandbox_output_scope;
use infimount_core::agent_tasks::{agent_task_root, AgentTaskBrief, AgentTaskManifest};
use infimount_core::CoreError;
use sha2::{Digest, Sha256};

pub(crate) struct VerifiedStagedOutput<'a> {
    pub task_path: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NofollowImportSummary {
    pub file_count: usize,
    pub total_bytes: u64,
}

fn error(reason: &'static str) -> CoreError {
    CoreError::Config(reason.into())
}

fn component_cstring(segment: &OsStr) -> io::Result<CString> {
    CString::new(segment.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn open_directory(parent: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    let name = component_cstring(name)?;
    // SAFETY: name is a NUL-terminated path component and parent is an
    // owned, live descriptor. O_NOFOLLOW denies this component's symlinks.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat just returned a unique owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn open_absolute_root(root: &Path) -> Result<OwnedFd, CoreError> {
    if !root.is_absolute() {
        return Err(error("sandbox task workspace root must be absolute"));
    }
    // SAFETY: constant NUL-terminated absolute root, no relative traversal.
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(error("sandbox task root directory is inaccessible"));
    }
    // SAFETY: the open call returned an owned descriptor.
    let mut current = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut components = 0usize;
    for part in root.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_directory(&current, name).map_err(|_| {
                    error("sandbox task root has an inaccessible or linked component")
                })?;
                components += 1;
            }
            _ => return Err(error("sandbox task root has an unsafe component")),
        }
    }
    if components == 0 {
        return Err(error("sandbox task root cannot be filesystem root"));
    }
    Ok(current)
}

fn open_existing_chain(root: &OwnedFd, components: &[&str]) -> io::Result<Option<OwnedFd>> {
    let mut dir = root.try_clone()?;
    for part in components {
        match open_directory(&dir, OsStr::new(part)) {
            Ok(next) => dir = next,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Ok(Some(dir))
}

fn require_existing_chain(root: &OwnedFd, components: &[&str]) -> Result<OwnedFd, CoreError> {
    open_existing_chain(root, components)
        .map_err(|_| error("sandbox task output ancestor is linked or inaccessible"))?
        .ok_or_else(|| error("sandbox task output directory is missing"))
}

fn leaf_exists(dir: &OwnedFd, leaf: &str) -> io::Result<bool> {
    let leaf = component_cstring(OsStr::new(leaf))?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: live dir FD, valid NUL-terminated leaf and allocated stat buffer.
    let rc = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            leaf.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::NotFound {
        return Ok(false);
    }
    Err(err)
}

fn ensure_child_dir(dir: &OwnedFd, name: &str) -> io::Result<OwnedFd> {
    let name_c = component_cstring(OsStr::new(name))?;
    // SAFETY: parent is pinned and name is one safe validated component.
    let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), name_c.as_ptr(), 0o700) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::AlreadyExists {
            return Err(err);
        }
    }
    open_directory(dir, OsStr::new(name))
}

fn create_new_file(dir: &OwnedFd, name: &str) -> io::Result<File> {
    let leaf = component_cstring(OsStr::new(name))?;
    // SAFETY: relative leaf, pinned directory, no-follow and exclusive create.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned an owned file descriptor, now transferred to File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn verify_committed_bytes(
    file: &mut File,
    expected_bytes: u64,
    expected_hash: &str,
) -> io::Result<()> {
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut seen = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        seen = seen
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        if seen > expected_bytes {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        hasher.update(&buffer[..n]);
    }
    if seen != expected_bytes || format!("{:x}", hasher.finalize()) != expected_hash {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(())
}

/// Only the caller holding the trusted workspace/configuration lock may invoke
/// this primitive. The root is a *known* Local Filesystem task workspace root,
/// not an arbitrary path chosen by a sandbox or model. No caller currently
/// exports this method via Tauri IPC.
///
/// All candidate bytes/paths are verified BEFORE the first write. The output
/// directory must already exist beneath trusted tasks/<UUID>/outputs.
/// Existing files never change. Later write failures require manual cleanup.
pub(crate) fn ingest_reviewed_outputs_to_unix_workspace(
    trusted_workspace_root: &Path,
    trusted_manifest: &AgentTaskManifest,
    reviewed_brief: &AgentTaskBrief,
    candidate: &SandboxOutputCandidatePlan,
    staged: &[VerifiedStagedOutput<'_>],
) -> Result<NofollowImportSummary, CoreError> {
    verify_reviewed_sandbox_output_scope(trusted_manifest, reviewed_brief, candidate)?;

    if staged.len() != candidate.entries.len() {
        return Err(error("sandbox staged files do not match reviewed outputs"));
    }
    let mut by_path: HashMap<&str, &[u8]> = HashMap::with_capacity(staged.len());
    for file in staged {
        if by_path.insert(file.task_path, file.bytes).is_some() {
            return Err(error("sandbox staged output paths contain duplicates"));
        }
    }
    let mut preflight = Vec::with_capacity(candidate.entries.len());
    let mut total = 0u64;
    for entry in &candidate.entries {
        let bytes = by_path
            .remove(entry.path.as_str())
            .ok_or_else(|| error("sandbox staged file was not in reviewed outputs"))?;
        if bytes.len() as u64 != entry.byte_size
            || format!("{:x}", Sha256::digest(bytes)) != entry.sha256
        {
            return Err(error(
                "sandbox staged output bytes differ from reviewed hashes",
            ));
        }
        total = total
            .checked_add(entry.byte_size)
            .ok_or_else(|| error("sandbox staged output size overflow"))?;
        let relative = entry
            .path
            .strip_prefix("outputs/")
            .ok_or_else(|| error("sandbox output path is not confined"))?;
        let parts: Vec<&str> = relative.split('/').collect();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(error("sandbox output path is invalid"));
        }
        preflight.push((parts, bytes, entry));
    }
    if !by_path.is_empty() || total != candidate.total_bytes {
        return Err(error(
            "sandbox staged output inventory differs from candidate",
        ));
    }

    let workspace = open_absolute_root(trusted_workspace_root)?;
    let task_root = agent_task_root(&trusted_manifest.task_id)?;
    let task_outputs = format!("{task_root}/outputs");
    let task_parts: Vec<&str> = task_outputs.split('/').collect();
    let output_dir = require_existing_chain(&workspace, &task_parts)?;

    // Preflight destination absence and existing parent integrity before any
    // output or new child directory is created.
    for (parts, _, _) in &preflight {
        if let Some(parent) = open_existing_chain(&output_dir, &parts[..parts.len() - 1])
            .map_err(|_| error("sandbox output parent is a symlink or inaccessible"))?
        {
            if leaf_exists(&parent, parts[parts.len() - 1])
                .map_err(|_| error("sandbox destination state cannot be verified"))?
            {
                return Err(error("sandbox output destination already exists"));
            }
        }
    }

    for (parts, bytes, entry) in &preflight {
        let mut parent = output_dir
            .try_clone()
            .map_err(|_| CoreError::TransferCleanupRequired)?;
        for part in &parts[..parts.len() - 1] {
            parent =
                ensure_child_dir(&parent, part).map_err(|_| CoreError::TransferCleanupRequired)?;
        }
        let mut destination = create_new_file(&parent, parts[parts.len() - 1])
            .map_err(|_| CoreError::TransferCleanupRequired)?;
        if destination.write_all(bytes).is_err()
            || verify_committed_bytes(&mut destination, entry.byte_size, &entry.sha256).is_err()
        {
            // Created bytes might be partially committed. Never delete on a
            // potentially raced path, and require visible manual cleanup.
            return Err(CoreError::TransferCleanupRequired);
        }
    }
    Ok(NofollowImportSummary {
        file_count: candidate.entries.len(),
        total_bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_import::{
        validate_sandbox_output_listing, SandboxRemoteEntryKind, SandboxRemoteOutputEntry,
        SandboxRemoteOutputListing, SANDBOX_IMPORT_SCHEMA_VERSION,
    };
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    const ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn prepared() -> (TempDir, AgentTaskManifest, AgentTaskBrief) {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join(format!("tasks/{ID}/outputs"))).unwrap();
        let manifest = AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: ID.into(),
            title: "PRIVATE local task".into(),
            created_at: "2026-11-08T12:00:00Z".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_root: format!("tasks/{ID}"),
            outputs_directory: "outputs".into(),
            inputs: vec![AgentTaskInput {
                task_path: "inputs/sample.csv".into(),
                byte_size: 5,
                sha256: format!("{:x}", Sha256::digest(b"alpha")),
            }],
        };
        let brief = AgentTaskBrief {
            title: "Reviewed synthetic data".into(),
            objective: "Write only the two reviewed output files.".into(),
            requested_outputs: vec![
                "outputs/summary.md".into(),
                "outputs/nested/data.csv".into(),
            ],
        };
        (root, manifest, brief)
    }

    fn candidate() -> SandboxOutputCandidatePlan {
        validate_sandbox_output_listing(&SandboxRemoteOutputListing {
            schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
            entries: vec![
                SandboxRemoteOutputEntry {
                    path: "outputs/summary.md".into(),
                    kind: SandboxRemoteEntryKind::File,
                    byte_size: 7,
                    sha256: format!("{:x}", Sha256::digest(b"summary")),
                },
                SandboxRemoteOutputEntry {
                    path: "outputs/nested/data.csv".into(),
                    kind: SandboxRemoteEntryKind::File,
                    byte_size: 3,
                    sha256: format!("{:x}", Sha256::digest(b"a,b")),
                },
            ],
        })
        .unwrap()
    }

    fn staged() -> Vec<VerifiedStagedOutput<'static>> {
        vec![
            VerifiedStagedOutput {
                task_path: "outputs/summary.md",
                bytes: b"summary",
            },
            VerifiedStagedOutput {
                task_path: "outputs/nested/data.csv",
                bytes: b"a,b",
            },
        ]
    }

    #[test]
    fn exact_reviewed_outputs_are_written_create_only_and_rehashed_from_handles() {
        let (root, manifest, brief) = prepared();
        let result = ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &candidate(),
            &staged(),
        )
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.total_bytes, 10);
        assert_eq!(
            fs::read(root.path().join(format!("tasks/{ID}/outputs/summary.md"))).unwrap(),
            b"summary"
        );
        assert_eq!(
            fs::read(
                root.path()
                    .join(format!("tasks/{ID}/outputs/nested/data.csv"))
            )
            .unwrap(),
            b"a,b"
        );
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &candidate(),
            &staged()
        )
        .is_err());
    }

    #[test]
    fn changed_bytes_fail_before_any_write() {
        let (root, manifest, brief) = prepared();
        let mut files = staged();
        files[0].bytes = b"changed";
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &candidate(),
            &files
        )
        .is_err());
        assert!(!root
            .path()
            .join(format!("tasks/{ID}/outputs/summary.md"))
            .exists());
    }

    #[test]
    fn unexpected_output_never_expands_user_review() {
        let (root, manifest, brief) = prepared();
        let mut other = candidate();
        other.entries[0].path = "outputs/surprise.md".into();
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &other,
            &staged()
        )
        .is_err());
        assert!(!root
            .path()
            .join(format!("tasks/{ID}/outputs/summary.md"))
            .exists());
    }

    #[test]
    fn linked_parent_or_root_is_never_followed() {
        let (root, manifest, brief) = prepared();
        let outside = TempDir::new().unwrap();
        let nested = root.path().join(format!("tasks/{ID}/outputs/nested"));
        symlink(outside.path(), &nested).unwrap();
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &candidate(),
            &staged()
        )
        .is_err());
        assert!(!outside.path().join("data.csv").exists());

        let alias = root.path().join("linked_root");
        symlink(root.path(), &alias).unwrap();
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            &alias,
            &manifest,
            &brief,
            &candidate(),
            &staged()
        )
        .is_err());
    }

    #[test]
    fn existing_sentinel_blocks_import_without_overwriting_other_outputs() {
        let (root, manifest, brief) = prepared();
        let sentinel = root.path().join(format!("tasks/{ID}/outputs/summary.md"));
        fs::write(&sentinel, b"sentinel").unwrap();
        assert!(ingest_reviewed_outputs_to_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            &candidate(),
            &staged()
        )
        .is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"sentinel");
        assert!(!root
            .path()
            .join(format!("tasks/{ID}/outputs/nested/data.csv"))
            .exists());
    }
}
