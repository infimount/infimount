//! Fail-closed, provider-neutral validation of a remote sandbox's reported
//! output file listing. This is a PURE candidate validator, not an importer.
//!
//! Metadata from a sandbox is untrusted. A future trusted importer MUST inspect
//! real source file types (including symlinks/reparse points), rehash downloaded
//! bytes, hold workspace mutation locks, and use create-only verified copy.
//! A claimed "file" kind, supplied size, or SHA-256 is never proof of bytes.
use std::collections::HashSet;

use infimount_core::CoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::publication::{validate_output_task_path, validate_sha256};
use crate::review_evidence::{
    MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES, MAX_AGENT_TASK_OUTPUT_REVIEW_FILES,
};

pub const SANDBOX_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const SANDBOX_IMPORT_DOMAIN: &str = "infimount:v0.8.2:sandbox-output-listing:v1";
pub const MAX_SANDBOX_OUTPUT_LISTING_JSON_BYTES: usize = 4 * 1024 * 1024;

/// A remote process may lie about this field. The trusted downloader must use
/// an independently acquired no-follow file-type check before opening bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxRemoteEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxRemoteOutputEntry {
    /// Must be a portable, canonical child of the task's "outputs/" directory.
    pub path: String,
    pub kind: SandboxRemoteEntryKind,
    pub byte_size: u64,
    /// Declared lowercase SHA-256, never accepted as measured integrity proof.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxRemoteOutputListing {
    pub schema_version: u32,
    pub entries: Vec<SandboxRemoteOutputEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxOutputCandidatePlan {
    pub schema_version: u32,
    pub entries: Vec<SandboxRemoteOutputEntry>,
    pub total_bytes: u64,
    /// Digest over canonical, untrusted DECLARED metadata, not file bytes.
    pub listing_metadata_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalOutputListing<'a> {
    domain: &'static str,
    schema_version: u32,
    entries: &'a [SandboxRemoteOutputEntry],
    total_bytes: u64,
}

/// Bound JSON allocations BEFORE decoding remote-provided data and reject any
/// unspecified authority fields, unknown file kinds or unsupported versions.
pub fn parse_and_validate_sandbox_output_listing(
    bytes: &[u8],
) -> Result<SandboxOutputCandidatePlan, CoreError> {
    if bytes.is_empty() || bytes.len() > MAX_SANDBOX_OUTPUT_LISTING_JSON_BYTES {
        return Err(CoreError::Config(
            "remote sandbox output listing exceeds its bounded JSON envelope".into(),
        ));
    }
    let listing: SandboxRemoteOutputListing = serde_json::from_slice(bytes)?;
    validate_sandbox_output_listing(&listing)
}

/// Validate a candidate remote listing without touching the local filesystem
/// or granting any right to download, import, execute or publish files.
pub fn validate_sandbox_output_listing(
    listing: &SandboxRemoteOutputListing,
) -> Result<SandboxOutputCandidatePlan, CoreError> {
    if listing.schema_version != SANDBOX_IMPORT_SCHEMA_VERSION
        || listing.entries.is_empty()
        || listing.entries.len() > MAX_AGENT_TASK_OUTPUT_REVIEW_FILES
    {
        return Err(CoreError::Config(
            "remote sandbox output listing has unsupported version or file count".into(),
        ));
    }

    let mut total_bytes = 0_u64;
    let mut portable_paths = HashSet::with_capacity(listing.entries.len());
    for file in &listing.entries {
        if file.kind != SandboxRemoteEntryKind::File
            || validate_output_task_path(&file.path).is_err()
            || validate_sha256(&file.sha256).is_err()
            || !portable_remote_path(&file.path)
            || !portable_paths.insert(file.path.to_ascii_lowercase())
        {
            return Err(CoreError::Config(
                "remote sandbox contains non-file, unsafe or colliding output entries".into(),
            ));
        }
        total_bytes = total_bytes.checked_add(file.byte_size).ok_or_else(|| {
            CoreError::Config("remote sandbox output byte count overflowed".into())
        })?;
        if total_bytes > MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES {
            return Err(CoreError::Config(
                "remote sandbox output exceeds the reviewed 2 GiB limit".into(),
            ));
        }
    }

    // Reject file/directory identity conflicts independent of entry ordering.
    // A file named outputs/a cannot coexist with outputs/a/child.txt.
    for normalized in &portable_paths {
        for (i, ch) in normalized.char_indices() {
            if ch == '/' && portable_paths.contains(&normalized[..i]) {
                return Err(CoreError::Config(
                    "remote sandbox output contains a file/directory path conflict".into(),
                ));
            }
        }
    }

    let mut entries = listing.entries.clone();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let canonical = CanonicalOutputListing {
        domain: SANDBOX_IMPORT_DOMAIN,
        schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
        entries: &entries,
        total_bytes,
    };
    let bytes = serde_json::to_vec(&canonical)?;
    if bytes.len() > MAX_SANDBOX_OUTPUT_LISTING_JSON_BYTES {
        return Err(CoreError::Config(
            "remote sandbox output listing exceeds its bounded canonical envelope".into(),
        ));
    }

    Ok(SandboxOutputCandidatePlan {
        schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
        entries,
        total_bytes,
        listing_metadata_sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

/// First C1 release targets the ASCII portable intersection. This deliberately
/// rejects ambiguous Unicode-normalization, percent-decoding, Windows device
/// names, and non-portable suffixes pending per-backend qualification.
fn portable_remote_path(path: &str) -> bool {
    path.split('/').skip(1).all(|segment| {
        if segment.is_empty()
            || segment.len() > 255
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || segment.bytes().any(|byte| {
                !(b' '..=b'~').contains(&byte)
                    || matches!(
                        byte,
                        b'%' | b'<' | b'>' | b':' | b'"' | b'\\' | b'|' | b'?' | b'*'
                    )
            })
        {
            return false;
        }
        let stem = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
        !segment.eq_ignore_ascii_case(".git")
            && !segment.eq_ignore_ascii_case(".infimount")
            && !reserved_windows_stem(&stem)
    })
}

fn reserved_windows_stem(stem: &str) -> bool {
    let bytes = stem.as_bytes();
    matches!(stem, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (bytes.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && bytes[3].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> SandboxRemoteOutputEntry {
        SandboxRemoteOutputEntry {
            path: path.into(),
            kind: SandboxRemoteEntryKind::File,
            byte_size: size,
            sha256: "a".repeat(64),
        }
    }

    fn listing(entries: Vec<SandboxRemoteOutputEntry>) -> SandboxRemoteOutputListing {
        SandboxRemoteOutputListing {
            schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
            entries,
        }
    }

    #[test]
    fn canonical_listing_is_order_independent_and_never_authorizes_a_write() {
        let initial = listing(vec![
            file("outputs/z/report.md", 5),
            file("outputs/a.md", 7),
        ]);
        let plan = validate_sandbox_output_listing(&initial).unwrap();
        assert_eq!(plan.total_bytes, 12);
        assert_eq!(plan.entries[0].path, "outputs/a.md");
        assert_eq!(plan.listing_metadata_sha256.len(), 64);
        // Pinned cross-provider canonical metadata v1 JSON: 362 UTF-8 bytes.
        // This digest is independently reproduced, not an integrity proof.
        assert_eq!(
            plan.listing_metadata_sha256,
            "1617e491c7145b61eb7c11cd4e6c497c6b8f379fa75c6a0cda3ad4785af96f8e"
        );
        let mut reversed = initial;
        reversed.entries.reverse();
        assert_eq!(plan, validate_sandbox_output_listing(&reversed).unwrap());
        let json = serde_json::to_value(&plan).unwrap();
        assert!(json.get("workspaceId").is_none());
        assert!(json.get("destinationStorageId").is_none());
        assert!(json.get("sourceHostPath").is_none());
        assert!(json.get("authorization").is_none());
        assert!(json.get("listingMetadataSha256").is_some());

        reversed.entries[0].sha256 = "b".repeat(64);
        assert_ne!(
            plan.listing_metadata_sha256,
            validate_sandbox_output_listing(&reversed)
                .unwrap()
                .listing_metadata_sha256
        );
    }

    #[test]
    fn rejects_foreign_subtrees_traversal_and_portable_collisions() {
        for path in [
            "inputs/private.txt",
            "outputs/../private.txt",
            "outputs//report.md",
            "outputs/a\\b.md",
            "outputs/%2e%2e/report.md",
            "outputs/C:/secret",
            "outputs/AUX.txt",
            "outputs/COM1",
            "outputs/COM0.txt",
            "outputs/lpt0",
            "outputs/CONIN$",
            "outputs/conout$.txt",
            "outputs/trailing.",
            "outputs/trailing ",
            "outputs/café.md",
            "outputs/.git/config",
            "/outputs/absolute.md",
        ] {
            assert!(
                validate_sandbox_output_listing(&listing(vec![file(path, 1)])).is_err(),
                "{path} should be rejected"
            );
        }
        for pair in [
            ["outputs/a.txt", "outputs/A.TXT"],
            ["outputs/report", "outputs/report/sub.txt"],
        ] {
            let candidates = listing(vec![file(pair[0], 1), file(pair[1], 2)]);
            assert!(
                validate_sandbox_output_listing(&candidates).is_err(),
                "colliding paths must be rejected"
            );
        }
    }

    #[test]
    fn refuses_symlinks_directories_and_unsupported_json_fields() {
        for kind in [
            SandboxRemoteEntryKind::Symlink,
            SandboxRemoteEntryKind::Directory,
            SandboxRemoteEntryKind::Other,
        ] {
            let mut claimed = file("outputs/a.md", 5);
            claimed.kind = kind;
            assert!(validate_sandbox_output_listing(&listing(vec![claimed])).is_err());
        }
        assert!(parse_and_validate_sandbox_output_listing(
            br#"{"schemaVersion":1,"entries":[{"path":"outputs/a.md","kind":"file","byteSize":5,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","approved":true}]}"#,
        )
        .is_err());
        assert!(parse_and_validate_sandbox_output_listing(
            br#"{"schemaVersion":999,"entries":[]}"#,
        )
        .is_err());
        assert!(parse_and_validate_sandbox_output_listing(
            br#"{"schemaVersion":1,"entries":[{"path":"outputs/a.md","kind":"socket","byteSize":5,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
        )
        .is_err());
    }

    #[test]
    fn enforces_file_and_byte_limits_before_import() {
        assert!(validate_sandbox_output_listing(&listing(vec![])).is_err());
        let too_many = (0..=MAX_AGENT_TASK_OUTPUT_REVIEW_FILES)
            .map(|index| file(&format!("outputs/{index}.txt"), 1))
            .collect();
        assert!(validate_sandbox_output_listing(&listing(too_many)).is_err());
        let over_size = listing(vec![
            file("outputs/a", MAX_AGENT_TASK_OUTPUT_REVIEW_BYTES),
            file("outputs/b", 1),
        ]);
        assert!(validate_sandbox_output_listing(&over_size).is_err());
        let overflow = listing(vec![file("outputs/a", u64::MAX), file("outputs/b", 1)]);
        assert!(validate_sandbox_output_listing(&overflow).is_err());
        let mut bad = file("outputs/a.md", 1);
        bad.sha256 = "A".repeat(64);
        assert!(validate_sandbox_output_listing(&listing(vec![bad])).is_err());
        let excessive_json = vec![b' '; MAX_SANDBOX_OUTPUT_LISTING_JSON_BYTES + 1];
        assert!(parse_and_validate_sandbox_output_listing(&excessive_json).is_err());
    }
}
