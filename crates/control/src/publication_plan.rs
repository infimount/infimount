//! Wire-compatible reviewed publication plan, preview fingerprint and receipt
//! contracts shared by trusted desktop and future headless entrypoints.
//!
//! A preview fingerprint detects staleness. It is not user approval or a
//! capability token and must never independently authorize publication.
use infimount_core::CoreError;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::publication::AgentTaskPublishConflictPolicy;

pub const AGENT_TASK_PUBLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskPublishAction {
    Create,
    Rename,
    Conflict,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationPlanFile {
    pub task_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub destination_path: String,
    pub action: AgentTaskPublishAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublishReceipt {
    pub schema_version: u32,
    pub publication_id: String,
    pub task_id: String,
    pub workspace_id: String,
    pub published_at: String,
    pub destination_storage_id: String,
    pub destination_storage_name: String,
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
    pub files: Vec<AgentTaskPublicationPlanFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskPublicationFingerprint<'a> {
    pub schema_version: u32,
    pub workspace_id: &'a str,
    pub task_id: &'a str,
    pub workspace_namespace: &'a str,
    pub destination_storage_id: &'a str,
    pub destination_namespace: &'a str,
    pub destination_dir: &'a str,
    pub conflict_policy: &'a AgentTaskPublishConflictPolicy,
    pub files: &'a [AgentTaskPublicationPlanFile],
}

/// Compute the legacy JSON-byte SHA-256 preview fingerprint. Caller must
/// re-plan and compare it under the existing publication locks before apply.
pub fn publication_preview_fingerprint(
    fingerprint: &AgentTaskPublicationFingerprint<'_>,
) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(fingerprint)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_file() -> AgentTaskPublicationPlanFile {
        AgentTaskPublicationPlanFile {
            task_path: "outputs/report.md".into(),
            byte_size: 3,
            sha256: "a".repeat(64),
            destination_path: "exports/report.md".into(),
            action: AgentTaskPublishAction::Create,
        }
    }

    #[test]
    fn preview_token_matches_legacy_json_byte_fingerprint() {
        let files = vec![plan_file()];
        let fingerprint = AgentTaskPublicationFingerprint {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            workspace_id: "workspace",
            task_id: "task",
            workspace_namespace: "workspace-ns",
            destination_storage_id: "destination",
            destination_namespace: "destination-ns",
            destination_dir: "exports",
            conflict_policy: &AgentTaskPublishConflictPolicy::Fail,
            files: &files,
        };
        let original = publication_preview_fingerprint(&fingerprint).unwrap();
        assert_eq!(
            original,
            "1a71f64f1b06bd6612509f1631f359cc2314bc6250a0e5d49a73a28d21680433"
        );

        let mut changed = files.clone();
        changed[0].sha256 = "b".repeat(64);
        let changed_fingerprint = AgentTaskPublicationFingerprint {
            files: &changed,
            ..fingerprint
        };
        assert_ne!(
            original,
            publication_preview_fingerprint(&changed_fingerprint).unwrap()
        );
    }

    #[test]
    fn preview_fingerprint_binds_destination_namespace_and_conflict_policy() {
        let files = vec![plan_file()];
        let token_for = |namespace: &str, policy: &AgentTaskPublishConflictPolicy| {
            let fingerprint = AgentTaskPublicationFingerprint {
                schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
                workspace_id: "workspace",
                task_id: "task",
                workspace_namespace: "workspace-ns",
                destination_storage_id: "destination",
                destination_namespace: namespace,
                destination_dir: "exports",
                conflict_policy: policy,
                files: &files,
            };
            publication_preview_fingerprint(&fingerprint).unwrap()
        };
        let original = token_for("destination-ns", &AgentTaskPublishConflictPolicy::Fail);
        assert_ne!(
            original,
            token_for(
                "other-destination-ns",
                &AgentTaskPublishConflictPolicy::Fail
            )
        );
        assert_ne!(
            original,
            token_for("destination-ns", &AgentTaskPublishConflictPolicy::Rename)
        );
    }

    #[test]
    fn receipt_and_action_json_remain_wire_compatible() {
        let receipt = AgentTaskPublishReceipt {
            schema_version: AGENT_TASK_PUBLICATION_SCHEMA_VERSION,
            publication_id: "publication".into(),
            task_id: "task".into(),
            workspace_id: "workspace".into(),
            published_at: "2026-09-08T00:00:00Z".into(),
            destination_storage_id: "destination-id".into(),
            destination_storage_name: "Published".into(),
            destination_dir: "exports".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Rename,
            files: vec![plan_file()],
        };
        let value = serde_json::to_value(receipt).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["conflictPolicy"], "rename");
        assert_eq!(value["files"][0]["taskPath"], "outputs/report.md");
        assert_eq!(value["files"][0]["action"], "create");
        assert!(value.get("sourceStorageId").is_none());
        assert!(value.get("hostPath").is_none());

        assert_eq!(
            serde_json::to_value(AgentTaskPublishAction::Conflict).unwrap(),
            "conflict"
        );
    }
}
