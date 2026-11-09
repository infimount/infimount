//! Experimental no-follow verification of prepared Unix Agent Task inputs.
//!
//! NOT a network uploader, Tauri/MCP tool or export approval issuer.
//! The trusted future caller MUST hold registry/workspace mutation locks,
//! verify the task-to-workspace binding and obtain separate consent.
//!
//! All file reads use pinned no-follow file descriptors. The full input bytes
//! are zeroized after the offline hosted request is constructed.
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use infimount_core::agent_tasks::{
    agent_task_root, AgentTaskBrief, AgentTaskManifest, AGENT_TASK_MANIFEST_FILE,
};
use infimount_core::CoreError;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::openai_hosted_request::{
    build_openai_hosted_offline_request, OpenAiHostedOfflineSessionRequest,
    ReviewedSandboxInputBytes, OPENAI_HOSTED_INLINE_MAX_BYTES, OPENAI_HOSTED_INLINE_MAX_FILES,
    OPENAI_HOSTED_INLINE_MAX_FILE_BYTES,
};
use crate::sandbox_bundle::{build_sandbox_task_bundle, SandboxTaskBundle};
use crate::sandbox_local_ingest::{open_absolute_root, require_existing_chain};
use serde::Serialize;

const MAX_TRUSTED_TASK_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

fn fail() -> CoreError {
    CoreError::Config(
        "prepared Agent Task bytes could not be verified with no-follow handles".into(),
    )
}

/// Deny FIFOs, devices, symlinks, hardlinks, oversized files and changed bytes.
/// Use O_NONBLOCK so a substituted FIFO cannot hang a trusted export preflight.
fn read_one_regular_file(
    root: &OwnedFd,
    components: &[&str],
    max_bytes: u64,
) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    if components.is_empty() || max_bytes == 0 {
        return Err(fail());
    }
    let parent = require_existing_chain(root, &components[..components.len() - 1])?;
    let leaf = CString::new(components[components.len() - 1].as_bytes()).map_err(|_| fail())?;
    // SAFETY: parent is a pinned live descriptor and leaf is a single
    // NUL-terminated path component. O_NOFOLLOW disallows symlink leaves.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(fail());
    }
    // SAFETY: openat returned a uniquely owned descriptor transferred to File.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|_| fail())?;
    if !meta.file_type().is_file() || meta.nlink() != 1 || meta.len() > max_bytes {
        return Err(fail());
    }
    let mut contents = Zeroizing::new(Vec::with_capacity(meta.len() as usize));
    (&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut contents)
        .map_err(|_| fail())?;
    if contents.len() as u64 != meta.len() {
        return Err(fail());
    }
    Ok(contents)
}

struct PreparedInputBytes {
    task_path: String,
    bytes: Zeroizing<Vec<u8>>,
}

/// Rehash all prepared task bytes from pinned Unix descriptors and construct
/// the already bounded, offline-only hosted request. The returned object
/// contains inline base64 user bytes, so it must never be logged or returned
/// to the browser/MCP. This does not contact an API or spend any money.
fn verify_prepared_input_handles(
    trusted_workspace_root: &Path,
    trusted_manifest: &AgentTaskManifest,
    separately_reviewed_export_brief: &AgentTaskBrief,
) -> Result<(SandboxTaskBundle, Vec<PreparedInputBytes>), CoreError> {
    let bundle = build_sandbox_task_bundle(trusted_manifest, separately_reviewed_export_brief)?;
    if bundle.requested_outputs.is_empty()
        || bundle.inputs.len() > OPENAI_HOSTED_INLINE_MAX_FILES
        || bundle.input_total_bytes > OPENAI_HOSTED_INLINE_MAX_BYTES as u64
        || bundle
            .inputs
            .iter()
            .any(|input| input.byte_size > OPENAI_HOSTED_INLINE_MAX_FILE_BYTES as u64)
    {
        return Err(fail());
    }

    let root = open_absolute_root(trusted_workspace_root)?;
    let task_root = agent_task_root(&trusted_manifest.task_id)?;
    let manifest_path = format!("{task_root}/{AGENT_TASK_MANIFEST_FILE}");
    let manifest_components: Vec<&str> = manifest_path.split('/').collect();
    let manifest_bytes =
        read_one_regular_file(&root, &manifest_components, MAX_TRUSTED_TASK_MANIFEST_BYTES)?;
    let actual_manifest: AgentTaskManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| fail())?;
    if actual_manifest != *trusted_manifest {
        return Err(fail());
    }

    let mut verified = Vec::with_capacity(bundle.inputs.len());
    let mut total = 0_u64;
    for item in &bundle.inputs {
        let relative = format!("{task_root}/{}", item.task_path);
        let components: Vec<&str> = relative.split('/').collect();
        let bytes = read_one_regular_file(
            &root,
            &components,
            OPENAI_HOSTED_INLINE_MAX_FILE_BYTES as u64,
        )?;
        if bytes.len() as u64 != item.byte_size
            || format!("{:x}", Sha256::digest(bytes.as_slice())) != item.sha256
        {
            return Err(fail());
        }
        total = total.checked_add(item.byte_size).ok_or_else(fail)?;
        verified.push(PreparedInputBytes {
            task_path: item.task_path.clone(),
            bytes,
        });
    }
    if total != bundle.input_total_bytes {
        return Err(fail());
    }
    Ok((bundle, verified))
}

/// A sanitized, bounded result of an actual native no-follow read.
/// This is a *fresh observation*, not a stable authorization, proof of user
/// presence, provider connection or permission to transfer those bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedHostedInputScope {
    pub input_file_count: usize,
    pub input_total_bytes: u64,
    pub bundle_metadata_sha256: String,
    pub verified_bytes: bool,
    pub export_authorized: bool,
}

/// Re-open every prepared byte through pinned no-follow Unix handles, verify
/// sizes and SHA-256 and immediately zeroize the in-memory copies. Returns
/// only metadata and a digest already bound to the manifest and reviewed
/// instructions. Future execution MUST verify again under native locks.
pub fn verify_prepared_hosted_input_scope(
    trusted_workspace_root: &Path,
    trusted_manifest: &AgentTaskManifest,
    separately_reviewed_export_brief: &AgentTaskBrief,
) -> Result<VerifiedHostedInputScope, CoreError> {
    let (bundle, _verified_bytes) = verify_prepared_input_handles(
        trusted_workspace_root,
        trusted_manifest,
        separately_reviewed_export_brief,
    )?;
    Ok(VerifiedHostedInputScope {
        input_file_count: bundle.inputs.len(),
        input_total_bytes: bundle.input_total_bytes,
        bundle_metadata_sha256: bundle.bundle_metadata_sha256,
        verified_bytes: true,
        export_authorized: false,
    })
}

/// Rehash actual input bytes again before constructing the offline request.
/// The caller must NEVER serialize this request to renderer or MCP output.
pub(crate) fn build_verified_hosted_request_from_unix_workspace(
    trusted_workspace_root: &Path,
    trusted_manifest: &AgentTaskManifest,
    separately_reviewed_export_brief: &AgentTaskBrief,
    model_id: &str,
) -> Result<OpenAiHostedOfflineSessionRequest, CoreError> {
    let (_bundle, verified) = verify_prepared_input_handles(
        trusted_workspace_root,
        trusted_manifest,
        separately_reviewed_export_brief,
    )?;
    let exact_refs: Vec<_> = verified
        .iter()
        .map(|entry| ReviewedSandboxInputBytes {
            task_path: &entry.task_path,
            bytes: entry.bytes.as_slice(),
        })
        .collect();
    // The standard provider request revalidates every byte and explicitly
    // includes disabled networking and a bounded hosted spending ceiling.
    // The temporary verified input buffers are zeroized on this function's
    // return, including when request construction fails.
    build_openai_hosted_offline_request(
        trusted_manifest,
        separately_reviewed_export_brief,
        &exact_refs,
        model_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    const ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";

    fn fixture() -> (TempDir, AgentTaskManifest, AgentTaskBrief) {
        let root = TempDir::new().unwrap();
        let task = root.path().join(format!("tasks/{ID}"));
        fs::create_dir_all(task.join("inputs")).unwrap();
        let manifest = AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: ID.into(),
            title: "PRIVATE original task".into(),
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
            title: "Reviewed synthetic export".into(),
            objective: "Analyze only the reviewed CSV file.".into(),
            requested_outputs: vec!["outputs/summary.md".into()],
        };
        fs::write(task.join("inputs/sample.csv"), b"alpha").unwrap();
        fs::write(
            task.join(AGENT_TASK_MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        (root, manifest, brief)
    }

    #[test]
    fn verified_input_bytes_construct_offline_network_denied_bounded_request() {
        let (root, manifest, brief) = fixture();
        let request = build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra",
        )
        .unwrap();
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["environment"]["network"]["access"], "disabled");
        assert_eq!(json["spend_control"]["limit"], 100);
        assert_eq!(
            json["environment"]["files"][0]["path"],
            "/workspace/inputs/sample.csv"
        );
        assert_eq!(json["environment"]["files"][0]["data"], "YWxwaGE=");
        assert!(json["input"]
            .as_str()
            .unwrap()
            .contains("outputs/summary.md"));
        let raw = serde_json::to_string(&json).unwrap();
        assert!(!raw.contains("PRIVATE original task"));
        assert!(!raw.contains(&manifest.workspace_id));
    }

    #[test]
    fn fresh_native_byte_scope_proves_actual_content_without_returning_any_bytes() {
        let (root, manifest, brief) = fixture();
        let scope = verify_prepared_hosted_input_scope(root.path(), &manifest, &brief).unwrap();
        assert_eq!(scope.input_file_count, 1);
        assert_eq!(scope.input_total_bytes, 5);
        assert!(scope.verified_bytes);
        assert!(!scope.export_authorized);
        assert_eq!(scope.bundle_metadata_sha256.len(), 64);
        let safe = serde_json::to_string(&scope).unwrap();
        assert!(!safe.contains("YWxwaGE="));
        assert!(!safe.contains("alpha"));
        assert!(!safe.contains("PRIVATE original task"));
        assert!(!safe.contains("inputs/sample.csv"));
        std::fs::write(
            root.path().join(format!("tasks/{ID}/inputs/sample.csv")),
            b"omega",
        )
        .unwrap();
        assert!(verify_prepared_hosted_input_scope(root.path(), &manifest, &brief).is_err());
    }

    #[test]
    fn rejects_same_size_source_mutation_and_manifest_substitution() {
        let (root, manifest, brief) = fixture();
        let task = root.path().join(format!("tasks/{ID}"));
        fs::write(task.join("inputs/sample.csv"), b"omega").unwrap();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());

        fs::write(task.join("inputs/sample.csv"), b"alpha").unwrap();
        let mut forged = manifest.clone();
        forged.title = "New forged title".into();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &forged,
            &brief,
            "gpt-6-astra"
        )
        .is_err());
    }

    #[test]
    fn rejects_symlink_leaf_symlinked_parent_and_hardlinks() {
        let (root, manifest, brief) = fixture();
        let task = root.path().join(format!("tasks/{ID}"));
        let source = task.join("inputs/sample.csv");
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("private.csv"), b"alpha").unwrap();
        fs::remove_file(&source).unwrap();
        symlink(outside.path().join("private.csv"), &source).unwrap();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());

        fs::remove_file(&source).unwrap();
        fs::write(&source, b"alpha").unwrap();
        let link = task.join("inputs/alias.csv");
        fs::hard_link(&source, &link).unwrap();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());

        fs::remove_file(&link).unwrap();
        fs::remove_file(&source).unwrap();
        fs::remove_dir(task.join("inputs")).unwrap();
        symlink(outside.path(), task.join("inputs")).unwrap();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());
    }

    #[test]
    fn rejects_symlinked_manifest_or_unreviewed_outputs() {
        let (root, manifest, mut brief) = fixture();
        brief.requested_outputs.clear();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());
        brief.requested_outputs.push("outputs/summary.md".into());
        let local_manifest = root
            .path()
            .join(format!("tasks/{ID}/{AGENT_TASK_MANIFEST_FILE}"));
        let external = TempDir::new().unwrap();
        fs::write(
            external.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::remove_file(&local_manifest).unwrap();
        symlink(external.path().join("manifest.json"), &local_manifest).unwrap();
        assert!(build_verified_hosted_request_from_unix_workspace(
            root.path(),
            &manifest,
            &brief,
            "gpt-6-astra"
        )
        .is_err());
    }
}
