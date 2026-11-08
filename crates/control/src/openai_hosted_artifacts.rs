//! Offline OpenAI-hosted artifact inventory and downloaded-byte verification.
//!
//! Provider artifact metadata is not authority over user-reviewed output paths.
//! The trusted transport must bind every page and file download to the
//! authenticated session. This module does NO network, disk, or cloud I/O.
use std::collections::{HashMap, HashSet};

use infimount_core::agent_tasks::{AgentTaskBrief, AgentTaskManifest};
use infimount_core::CoreError;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::openai_hosted_events::HostedSessionObservedOutcome;
use crate::sandbox_bundle::build_sandbox_task_bundle;
use crate::sandbox_import::{
    validate_sandbox_output_listing, SandboxOutputCandidatePlan, SandboxRemoteEntryKind,
    SandboxRemoteOutputEntry, SandboxRemoteOutputListing, SANDBOX_IMPORT_SCHEMA_VERSION,
};

pub const MAX_HOSTED_ARTIFACT_PAGE_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HOSTED_ARTIFACT_PAGES: usize = 100;
pub const MAX_HOSTED_ARTIFACT_FILES: usize = 100;
pub const MAX_HOSTED_ARTIFACT_FILE_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_HOSTED_ARTIFACT_TOTAL_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactPage {
    object: String,
    data: Vec<ApiArtifact>,
    first_id: Option<String>,
    last_id: Option<String>,
    has_more: bool,
}

#[derive(Deserialize)]
struct ApiArtifact {
    id: String,
    object: String,
    environment_id: String,
    session_id: String,
    turn_id: String,
    path: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedArtifactDescriptor {
    pub id: String,
    /// Canonical task-relative path, always under outputs/.
    pub task_path: String,
    pub size_bytes: u64,
}

/// Metadata-only result. Not verified file contents, execution evidence,
/// export authorization or permission to import or publish.
pub struct HostedArtifactPlan {
    files: Vec<HostedArtifactDescriptor>,
    total_bytes: u64,
}

impl HostedArtifactPlan {
    pub fn files(&self) -> &[HostedArtifactDescriptor] {
        &self.files
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

pub struct HostedArtifactInventory {
    expected_session_id: String,
    expected_turn_id: String,
    expected_environment_id: String,
    requested_outputs: Vec<String>,
    files: Vec<HostedArtifactDescriptor>,
    seen_ids: HashSet<String>,
    seen_paths: HashSet<String>,
    next_after: Option<String>,
    total_bytes: u64,
    pages: usize,
    complete: bool,
    failed: bool,
}

impl HostedArtifactInventory {
    /// Exact reviewed deliverables from a trusted local task and a separately
    /// reviewed export brief. A fresh hosted session is required for this pilot.
    pub fn new(
        trusted_manifest: &AgentTaskManifest,
        reviewed_export_brief: &AgentTaskBrief,
        session_id: &str,
        turn_id: &str,
        environment_id: &str,
    ) -> Result<Self, CoreError> {
        if !safe_id(session_id) || !safe_id(turn_id) || !safe_id(environment_id) {
            return Err(artifact_error());
        }
        let bundle = build_sandbox_task_bundle(trusted_manifest, reviewed_export_brief)?;
        if bundle.requested_outputs.is_empty()
            || bundle.requested_outputs.len() > MAX_HOSTED_ARTIFACT_FILES
        {
            return Err(artifact_error());
        }
        Ok(Self {
            expected_session_id: session_id.into(),
            expected_turn_id: turn_id.into(),
            expected_environment_id: environment_id.into(),
            requested_outputs: bundle.requested_outputs,
            files: Vec::new(),
            seen_ids: HashSet::new(),
            seen_paths: HashSet::new(),
            next_after: None,
            total_bytes: 0,
            pages: 0,
            complete: false,
            failed: false,
        })
    }

    /// Send this cursor in the next authenticated page request with order=asc.
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }

    /// requested_after is the actual cursor used by the trusted transport.
    /// Any page inconsistency permanently poisons this inventory.
    pub fn observe_page(
        &mut self,
        page_json: &[u8],
        requested_after: Option<&str>,
    ) -> Result<(), CoreError> {
        if self.failed
            || self.complete
            || self.pages >= MAX_HOSTED_ARTIFACT_PAGES
            || self.next_after.as_deref() != requested_after
            || page_json.is_empty()
            || page_json.len() > MAX_HOSTED_ARTIFACT_PAGE_JSON_BYTES
        {
            return self.reject();
        }
        let page: ArtifactPage = match serde_json::from_slice(page_json) {
            Ok(page) => page,
            Err(_) => return self.reject(),
        };
        if page.object != "list"
            || page.data.is_empty()
            || page.data.len() > MAX_HOSTED_ARTIFACT_FILES
            || self.files.len().saturating_add(page.data.len()) > MAX_HOSTED_ARTIFACT_FILES
            || (page.has_more && page.last_id.is_none())
        {
            return self.reject();
        }
        let mut pending = Vec::with_capacity(page.data.len());
        let mut seen_page_ids = HashSet::new();
        let mut seen_page_paths = HashSet::new();
        let mut new_total = self.total_bytes;
        for (index, entry) in page.data.iter().enumerate() {
            if !safe_id(&entry.id)
                || entry.object != "agent.session.artifact"
                || entry.session_id != self.expected_session_id
                || entry.turn_id != self.expected_turn_id
                || entry.environment_id != self.expected_environment_id
                || self.seen_ids.contains(&entry.id)
                || !seen_page_ids.insert(entry.id.as_str())
            {
                return self.reject();
            }
            if (index == 0 && page.first_id.as_deref() != Some(entry.id.as_str()))
                || (index == page.data.len() - 1
                    && page.last_id.as_deref() != Some(entry.id.as_str()))
            {
                return self.reject();
            }
            // The provider supplies an absolute path. Do not normalize it,
            // decode URL escapes, or accept a host-selected destination.
            let Some(task_path) = entry.path.strip_prefix("/workspace/") else {
                return self.reject();
            };
            if !task_path.starts_with("outputs/")
                || !self.requested_outputs.iter().any(|p| p == task_path)
                || self.seen_paths.contains(&task_path.to_ascii_lowercase())
                || !seen_page_paths.insert(task_path.to_ascii_lowercase())
                || entry.size_bytes > MAX_HOSTED_ARTIFACT_FILE_BYTES
            {
                return self.reject();
            }
            new_total = match new_total.checked_add(entry.size_bytes) {
                Some(value) if value <= MAX_HOSTED_ARTIFACT_TOTAL_BYTES => value,
                _ => return self.reject(),
            };
            pending.push(HostedArtifactDescriptor {
                id: entry.id.clone(),
                task_path: task_path.to_owned(),
                size_bytes: entry.size_bytes,
            });
        }
        self.seen_ids
            .extend(seen_page_ids.into_iter().map(str::to_owned));
        self.seen_paths.extend(seen_page_paths);
        self.total_bytes = new_total;
        self.files.extend(pending);
        self.pages += 1;
        if page.has_more {
            self.next_after = page.last_id;
        } else {
            self.next_after = None;
            self.complete = true;
        }
        Ok(())
    }

    /// Consume the one-shot inventory. Root-turn completion and the separate
    /// saved-item audit are mandatory *preconditions*, not evidence of bytes.
    pub fn finish(
        mut self,
        outcome: HostedSessionObservedOutcome,
    ) -> Result<HostedArtifactPlan, CoreError> {
        if self.failed
            || !self.complete
            || self.pages == 0
            || outcome != HostedSessionObservedOutcome::RootTurnCompletedUnverified
            || self.files.len() != self.requested_outputs.len()
        {
            return Err(artifact_error());
        }
        self.files.sort_by(|a, b| a.task_path.cmp(&b.task_path));
        if !self
            .files
            .iter()
            .map(|a| a.task_path.as_str())
            .eq(self.requested_outputs.iter().map(String::as_str))
        {
            return Err(artifact_error());
        }
        Ok(HostedArtifactPlan {
            files: self.files,
            total_bytes: self.total_bytes,
        })
    }

    fn reject(&mut self) -> Result<(), CoreError> {
        self.failed = true;
        Err(artifact_error())
    }
}

/// Actual bytes downloaded by a trusted client from the listed artifact ID.
/// The caller MUST authenticate every content request and stage those SAME
/// bytes in the isolated import source. No contents are retained by the plan.
pub struct DownloadedHostedArtifact<'a> {
    pub artifact_id: &'a str,
    pub bytes: &'a [u8],
}

/// Compute actual downloaded SHA-256 for every immutable artifact ID.
/// Provider listing has only size metadata, not an integrity fingerprint.
/// Returns the provider-neutral C1 candidate, which still requires separate
/// actual staged-byte rehashing and create-only, reviewed-scope import.
pub fn validate_downloaded_hosted_artifacts(
    plan: &HostedArtifactPlan,
    downloaded: &[DownloadedHostedArtifact<'_>],
) -> Result<SandboxOutputCandidatePlan, CoreError> {
    if plan.files.is_empty() || plan.files.len() != downloaded.len() {
        return Err(artifact_error());
    }
    let mut by_id: HashMap<&str, &[u8]> = HashMap::with_capacity(downloaded.len());
    for entry in downloaded {
        if !safe_id(entry.artifact_id) || by_id.insert(entry.artifact_id, entry.bytes).is_some() {
            return Err(artifact_error());
        }
    }
    let mut actual_total = 0_u64;
    let mut entries = Vec::with_capacity(plan.files.len());
    for artifact in &plan.files {
        let bytes = by_id
            .remove(artifact.id.as_str())
            .ok_or_else(artifact_error)?;
        if bytes.len() as u64 != artifact.size_bytes {
            return Err(artifact_error());
        }
        actual_total = actual_total
            .checked_add(artifact.size_bytes)
            .ok_or_else(artifact_error)?;
        entries.push(SandboxRemoteOutputEntry {
            path: artifact.task_path.clone(),
            kind: SandboxRemoteEntryKind::File,
            byte_size: artifact.size_bytes,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
    }
    if !by_id.is_empty() || actual_total != plan.total_bytes {
        return Err(artifact_error());
    }
    validate_sandbox_output_listing(&SandboxRemoteOutputListing {
        schema_version: SANDBOX_IMPORT_SCHEMA_VERSION,
        entries,
    })
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn artifact_error() -> CoreError {
    CoreError::Config(
        "hosted artifact inventory or downloaded bytes failed offline verification".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_reviewed_ingest::ingest_reviewed_sandbox_outputs_into_memory;
    use infimount_core::agent_tasks::{AgentTaskInput, AGENT_TASK_SCHEMA_VERSION};
    use opendal::Operator;
    use serde_json::{json, Value};

    const TASK_ID: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
    const ROOT: HostedSessionObservedOutcome =
        HostedSessionObservedOutcome::RootTurnCompletedUnverified;

    fn manifest() -> AgentTaskManifest {
        AgentTaskManifest {
            schema_version: AGENT_TASK_SCHEMA_VERSION,
            task_id: TASK_ID.into(),
            title: "PRIVATE input data source".into(),
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
            objective: "Write the two approved deliverables.".into(),
            requested_outputs: vec!["outputs/report.md".into(), "outputs/data.csv".into()],
        }
    }

    fn artifact(id: &str, path: &str, n: usize) -> Value {
        json!({
            "id": id,
            "object": "agent.session.artifact",
            "environment_id": "env_test",
            "session_id": "sess_test",
            "turn_id": "turn_test",
            "path": path,
            "size_bytes": n,
            "created_at": 1780000000
        })
    }

    fn page(entries: Vec<Value>, more: bool) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "object": "list",
            "first_id": entries.first().and_then(|e| e["id"].as_str()),
            "last_id": entries.last().and_then(|e| e["id"].as_str()),
            "has_more": more,
            "data": entries
        }))
        .unwrap()
    }

    fn inventory() -> HostedArtifactInventory {
        HostedArtifactInventory::new(&manifest(), &brief(), "sess_test", "turn_test", "env_test")
            .unwrap()
    }

    fn two_page_plan() -> HostedArtifactPlan {
        let mut inv = inventory();
        inv.observe_page(
            &page(
                vec![artifact("art_report", "/workspace/outputs/report.md", 7)],
                true,
            ),
            None,
        )
        .unwrap();
        assert_eq!(inv.next_after(), Some("art_report"));
        inv.observe_page(
            &page(
                vec![artifact("art_data", "/workspace/outputs/data.csv", 3)],
                false,
            ),
            Some("art_report"),
        )
        .unwrap();
        inv.finish(ROOT).unwrap()
    }

    fn memory() -> Operator {
        Operator::new(opendal::services::Memory::default()).unwrap()
    }

    #[tokio::test]
    async fn verified_downloads_import_exact_reviewed_outputs_create_only() {
        let plan = two_page_plan();
        assert_eq!(plan.total_bytes(), 10);
        assert_eq!(plan.files().len(), 2);
        let downloaded = [
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"summary",
            },
            DownloadedHostedArtifact {
                artifact_id: "art_data",
                bytes: b"a,b",
            },
        ];
        let candidate = validate_downloaded_hosted_artifacts(&plan, &downloaded).unwrap();
        let staged = memory();
        let dest = memory();
        staged
            .write("outputs/report.md", b"summary".to_vec())
            .await
            .unwrap();
        staged
            .write("outputs/data.csv", b"a,b".to_vec())
            .await
            .unwrap();
        let result = ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &candidate,
            &staged,
            &dest,
        )
        .await
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(
            dest.read(&format!("tasks/{TASK_ID}/outputs/report.md"))
                .await
                .unwrap()
                .to_vec(),
            b"summary"
        );
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &candidate,
            &staged,
            &dest,
        )
        .await
        .is_err());
    }

    #[test]
    fn mismatched_download_ids_and_size_are_rejected() {
        let plan = two_page_plan();
        let invalid = [
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"changed",
            },
            DownloadedHostedArtifact {
                artifact_id: "art_unknown",
                bytes: b"a,b",
            },
        ];
        assert!(validate_downloaded_hosted_artifacts(&plan, &invalid).is_err());
        let short = [
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"short",
            },
            DownloadedHostedArtifact {
                artifact_id: "art_data",
                bytes: b"a,b",
            },
        ];
        assert!(validate_downloaded_hosted_artifacts(&plan, &short).is_err());
        let duplicate = [
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"summary",
            },
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"a,b",
            },
        ];
        assert!(validate_downloaded_hosted_artifacts(&plan, &duplicate).is_err());
    }

    #[tokio::test]
    async fn staged_byte_substitution_after_download_does_not_import() {
        let plan = two_page_plan();
        let downloaded = [
            DownloadedHostedArtifact {
                artifact_id: "art_report",
                bytes: b"summary",
            },
            DownloadedHostedArtifact {
                artifact_id: "art_data",
                bytes: b"a,b",
            },
        ];
        let candidate = validate_downloaded_hosted_artifacts(&plan, &downloaded).unwrap();
        let staged = memory();
        let dest = memory();
        staged
            .write("outputs/report.md", b"changed".to_vec())
            .await
            .unwrap();
        staged
            .write("outputs/data.csv", b"a,b".to_vec())
            .await
            .unwrap();
        assert!(ingest_reviewed_sandbox_outputs_into_memory(
            &manifest(),
            &brief(),
            &candidate,
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

    #[test]
    fn extra_missing_and_cross_context_artifacts_fail_closed() {
        let mut inv = inventory();
        assert!(inv
            .observe_page(
                &page(
                    vec![
                        artifact("art_1", "/workspace/outputs/report.md", 7),
                        artifact("art_2", "/workspace/outputs/data.csv", 3),
                        artifact("art_3", "/workspace/outputs/secret.txt", 8)
                    ],
                    false
                ),
                None
            )
            .is_err());

        let mut inv = inventory();
        inv.observe_page(
            &page(
                vec![artifact("art_1", "/workspace/outputs/report.md", 7)],
                false,
            ),
            None,
        )
        .unwrap();
        assert!(inv.finish(ROOT).is_err());

        for field in ["turn_id", "session_id", "environment_id"] {
            let mut entry = artifact("art_1", "/workspace/outputs/report.md", 7);
            entry[field] = json!("other");
            let mut inv = inventory();
            assert!(inv.observe_page(&page(vec![entry], false), None).is_err());
        }
    }

    #[test]
    fn unsafe_path_duplicate_ids_and_cursor_drift_fail_closed() {
        for path in [
            "/workspace/outputs/../secret",
            "/workspace/outputs/REPORT.md",
            "/workspace/inputs/sample.csv",
            "outputs/report.md",
            "/workspace/outputs/report.md/extra",
            "/workspace/outputs/report.md%2fsecret",
            "/workspace/outputs\\report.md",
        ] {
            let mut inv = inventory();
            assert!(inv
                .observe_page(&page(vec![artifact("art_1", path, 7)], false), None)
                .is_err());
        }
        let mut inv = inventory();
        assert!(inv
            .observe_page(
                &page(
                    vec![
                        artifact("art_1", "/workspace/outputs/report.md", 7),
                        artifact("art_1", "/workspace/outputs/data.csv", 3)
                    ],
                    false
                ),
                None
            )
            .is_err());
        let mut inv = inventory();
        inv.observe_page(
            &page(
                vec![artifact("art_1", "/workspace/outputs/report.md", 7)],
                true,
            ),
            None,
        )
        .unwrap();
        assert!(inv
            .observe_page(
                &page(
                    vec![artifact("art_2", "/workspace/outputs/data.csv", 3)],
                    false
                ),
                Some("wrong")
            )
            .is_err());
        assert!(inv.finish(ROOT).is_err());
    }

    #[test]
    fn maximum_provider_output_size_and_terminal_state_are_enforced() {
        let mut inv = inventory();
        assert!(inv
            .observe_page(
                &page(
                    vec![artifact(
                        "art_1",
                        "/workspace/outputs/report.md",
                        MAX_HOSTED_ARTIFACT_FILE_BYTES as usize + 1
                    )],
                    false
                ),
                None
            )
            .is_err());
        let mut inv = inventory();
        inv.observe_page(
            &page(
                vec![
                    artifact("art_1", "/workspace/outputs/report.md", 7),
                    artifact("art_2", "/workspace/outputs/data.csv", 3),
                ],
                false,
            ),
            None,
        )
        .unwrap();
        assert!(inv.finish(HostedSessionObservedOutcome::Pending).is_err());
    }

    #[test]
    fn malformed_and_oversize_page_poison_inventory() {
        let mut inv = inventory();
        assert!(inv
            .observe_page(&vec![b' '; MAX_HOSTED_ARTIFACT_PAGE_JSON_BYTES + 1], None)
            .is_err());
        assert!(inv
            .observe_page(
                &page(
                    vec![artifact("art_1", "/workspace/outputs/report.md", 7)],
                    false
                ),
                None
            )
            .is_err());
    }
}
