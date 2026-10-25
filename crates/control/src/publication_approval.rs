//! Wire and binding contract for a future *server-held* publication approval.
//!
//! A serialized record is not authority by itself. Callers must fetch it from
//! a protected, transactional store outside agent-visible paths, authenticate
//! the actor, lock the authoritative publication context, replan all bytes and
//! destination state, and consume the grant exactly once before any write.
//! This module deliberately creates no grant store, IPC or publish endpoint.
use std::collections::HashSet;

use infimount_core::CoreError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::publication::{
    normalize_destination_dir, validate_output_task_path, validate_sha256,
    AgentTaskPublishConflictPolicy, MAX_AGENT_TASK_DESTINATION_PATH_BYTES,
    MAX_AGENT_TASK_PUBLISH_FILES,
};
use crate::publication_plan::{AgentTaskPublicationPlanFile, AgentTaskPublishAction};

pub const PUBLICATION_APPROVAL_SCHEMA_VERSION: u32 = 1;
pub const MAX_PUBLICATION_APPROVAL_LIFETIME_MS: u64 = 15 * 60 * 1000;

/// The private record that a future trusted approval store will persist.
/// Never expose this record (particularly its one-use nonce) to an agent.
/// Deserialize only after loading it from an authenticated protected store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicationApprovalRecord {
    pub schema_version: u32,
    pub grant_id: String,
    pub actor_id: String,
    pub workspace_id: String,
    pub task_id: String,
    pub destination_storage_id: String,
    pub destination_namespace: String,
    pub destination_revision: u64,
    pub destination_dir: String,
    pub conflict_policy: AgentTaskPublishConflictPolicy,
    pub preview_fingerprint: String,
    pub files: Vec<AgentTaskPublicationPlanFile>,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at_unix_ms: Option<u64>,
}

/// Fresh trusted values re-derived while holding configuration/workspace locks.
/// No field in this snapshot should be sourced from agent-supplied approval JSON.
pub struct PublicationApprovalSnapshot<'a> {
    pub workspace_id: &'a str,
    pub task_id: &'a str,
    pub destination_storage_id: &'a str,
    pub destination_namespace: &'a str,
    pub destination_revision: u64,
    pub destination_dir: &'a str,
    pub conflict_policy: &'a AgentTaskPublishConflictPolicy,
    pub preview_fingerprint: &'a str,
    pub files: &'a [AgentTaskPublicationPlanFile],
    pub conflict_count: usize,
}

/// Check binding only. SUCCESS IS NOT PERMISSION TO PUBLISH: a protected store
/// must still atomically consume an authenticated grant under the same locks.
/// No API accepts a grant supplied by an agent as write authority.
pub fn validate_protected_approval_binding(
    record: &PublicationApprovalRecord,
    snapshot: &PublicationApprovalSnapshot<'_>,
    authenticated_actor_id: &str,
    now_unix_ms: u64,
) -> Result<(), CoreError> {
    if record.schema_version != PUBLICATION_APPROVAL_SCHEMA_VERSION
        || Uuid::parse_str(&record.grant_id).is_err()
        || Uuid::parse_str(&record.nonce).is_err()
        || Uuid::parse_str(&record.task_id).is_err()
        || Uuid::parse_str(&record.workspace_id).is_err()
        || record.actor_id.is_empty()
        || record.actor_id.len() > 128
        || record.actor_id.trim() != record.actor_id
        || record.actor_id.chars().any(char::is_control)
        || validate_sha256(&record.preview_fingerprint).is_err()
        || record.destination_revision == 0
        || record.destination_revision == u64::MAX
        || record.files.is_empty()
        || record.files.len() > MAX_AGENT_TASK_PUBLISH_FILES
        || record.issued_at_unix_ms >= record.expires_at_unix_ms
        || record.expires_at_unix_ms - record.issued_at_unix_ms
            > MAX_PUBLICATION_APPROVAL_LIFETIME_MS
        || now_unix_ms < record.issued_at_unix_ms
        || now_unix_ms >= record.expires_at_unix_ms
        || record.consumed_at_unix_ms.is_some()
        || record.revoked_at_unix_ms.is_some()
    {
        return Err(CoreError::Config(
            "publication approval is invalid, expired, revoked or already consumed".into(),
        ));
    }
    // Check the sealed record's own structural consistency. Matching a
    // snapshot alone is insufficient if a future issuer builds that snapshot
    // incorrectly, or a deserialized record contains contradictory actions.
    if normalize_destination_dir(&record.destination_dir)
        .map_or(true, |normalized| normalized != record.destination_dir)
    {
        return Err(CoreError::Config(
            "publication approval destination directory is invalid".into(),
        ));
    }
    let mut seen_task_paths = HashSet::with_capacity(record.files.len());
    let mut seen_destination_paths = HashSet::with_capacity(record.files.len());
    for (i, file) in record.files.iter().enumerate() {
        let under_approved_directory = record.destination_dir.is_empty()
            || file
                .destination_path
                .starts_with(&format!("{}/", record.destination_dir));
        if validate_output_task_path(&file.task_path).is_err()
            || validate_sha256(&file.sha256).is_err()
            || !valid_relative_destination_path(&file.destination_path)
            || !under_approved_directory
            || file.action == AgentTaskPublishAction::Conflict
            || (record.conflict_policy == AgentTaskPublishConflictPolicy::Fail
                && file.action != AgentTaskPublishAction::Create)
            || !seen_task_paths.insert(file.task_path.to_lowercase())
            || !seen_destination_paths.insert(file.destination_path.to_lowercase())
            || (i > 0 && record.files[i - 1].task_path >= file.task_path)
        {
            return Err(CoreError::Config(
                "publication approval has malformed reviewed output bindings".into(),
            ));
        }
    }

    if snapshot.conflict_count != 0
        || authenticated_actor_id != record.actor_id
        || record.workspace_id != snapshot.workspace_id
        || record.task_id != snapshot.task_id
        || record.destination_storage_id != snapshot.destination_storage_id
        || record.destination_namespace != snapshot.destination_namespace
        || record.destination_revision != snapshot.destination_revision
        || record.destination_dir != snapshot.destination_dir
        || &record.conflict_policy != snapshot.conflict_policy
        || record.preview_fingerprint != snapshot.preview_fingerprint
        || record.files != snapshot.files
    {
        return Err(CoreError::Config(
            "publication approval no longer matches the trusted current plan".into(),
        ));
    }
    Ok(())
}

fn valid_relative_destination_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_AGENT_TASK_DESTINATION_PATH_BYTES
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication_plan::AgentTaskPublishAction;

    const WORKSPACE: &str = "f8f47aa7-702d-4fd6-8815-84cbdf3b3127";
    const TASK: &str = "81f08176-86e4-40ec-a9a4-a219c4c9b454";
    const GRANT: &str = "421ea05a-95c7-4ef3-a7e3-6b821a0e4ac3";
    const NONCE: &str = "c7da7279-1fc9-4f7f-ad1e-114706628bf1";

    fn record() -> PublicationApprovalRecord {
        PublicationApprovalRecord {
            schema_version: PUBLICATION_APPROVAL_SCHEMA_VERSION,
            grant_id: GRANT.into(),
            actor_id: "local-human-actor".into(),
            workspace_id: WORKSPACE.into(),
            task_id: TASK.into(),
            destination_storage_id: "destination".into(),
            destination_namespace: "destination-ns".into(),
            destination_revision: 42,
            destination_dir: "exports".into(),
            conflict_policy: AgentTaskPublishConflictPolicy::Fail,
            preview_fingerprint: "a".repeat(64),
            files: vec![AgentTaskPublicationPlanFile {
                task_path: "outputs/report.md".into(),
                byte_size: 5,
                sha256: "b".repeat(64),
                destination_path: "exports/report.md".into(),
                action: AgentTaskPublishAction::Create,
            }],
            issued_at_unix_ms: 1_000_000,
            expires_at_unix_ms: 1_300_000,
            nonce: NONCE.into(),
            consumed_at_unix_ms: None,
            revoked_at_unix_ms: None,
        }
    }

    fn snapshot<'a>(record: &'a PublicationApprovalRecord) -> PublicationApprovalSnapshot<'a> {
        PublicationApprovalSnapshot {
            workspace_id: &record.workspace_id,
            task_id: &record.task_id,
            destination_storage_id: &record.destination_storage_id,
            destination_namespace: &record.destination_namespace,
            destination_revision: record.destination_revision,
            destination_dir: &record.destination_dir,
            conflict_policy: &record.conflict_policy,
            preview_fingerprint: &record.preview_fingerprint,
            files: &record.files,
            conflict_count: 0,
        }
    }

    fn valid(
        record: &PublicationApprovalRecord,
        current: &PublicationApprovalSnapshot<'_>,
    ) -> bool {
        validate_protected_approval_binding(record, current, "local-human-actor", 1_050_000).is_ok()
    }

    #[test]
    fn exact_live_binding_only_is_valid() {
        let record = record();
        let current = snapshot(&record);
        assert!(valid(&record, &current));
        let other_actor =
            validate_protected_approval_binding(&record, &current, "agent-actor", 1_050_000);
        assert!(other_actor.is_err());
        let expired =
            validate_protected_approval_binding(&record, &current, "local-human-actor", 1_300_000);
        assert!(expired.is_err());
    }

    #[test]
    fn consumed_revoked_malformed_or_long_lived_grants_fail() {
        let mut record = record();
        record.consumed_at_unix_ms = Some(1_050_000);
        assert!(!valid(&record, &snapshot(&record)));
        record.consumed_at_unix_ms = None;
        record.revoked_at_unix_ms = Some(1_040_000);
        assert!(!valid(&record, &snapshot(&record)));
        record.revoked_at_unix_ms = None;
        record.expires_at_unix_ms = 2_000_000;
        assert!(!valid(&record, &snapshot(&record)));
        record.expires_at_unix_ms = 1_300_000;
        record.nonce = "not-a-nonce".into();
        assert!(!valid(&record, &snapshot(&record)));
    }

    #[test]
    fn invalid_or_saturated_destination_revisions_fail_closed() {
        let mut grant = record();
        grant.destination_revision = 0;
        assert!(!valid(&grant, &snapshot(&grant)));
        grant.destination_revision = u64::MAX;
        assert!(!valid(&grant, &snapshot(&grant)));
    }

    #[test]
    fn destination_revision_namespace_conflicts_or_files_changed_fail() {
        let record = record();
        let mut current = snapshot(&record);
        current.destination_revision += 1;
        assert!(!valid(&record, &current));
        let mut current = snapshot(&record);
        current.destination_namespace = "different-ns";
        assert!(!valid(&record, &current));
        let mut current = snapshot(&record);
        current.conflict_count = 1;
        assert!(!valid(&record, &current));

        let mut files = record.files.clone();
        files[0].sha256 = "c".repeat(64);
        let mut current = snapshot(&record);
        current.files = &files;
        assert!(!valid(&record, &current));

        let mut files = record.files.clone();
        files[0].destination_path = "other/report.md".into();
        let mut current = snapshot(&record);
        current.files = &files;
        assert!(!valid(&record, &current));
    }

    #[test]
    fn contradictory_action_and_unsafe_destination_fail_even_with_matching_snapshot() {
        let mut grant = record();
        grant.files[0].action = AgentTaskPublishAction::Conflict;
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[0].action = AgentTaskPublishAction::Rename;
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.conflict_policy = AgentTaskPublishConflictPolicy::Rename;
        grant.files[0].destination_path = "../secret/report.md".into();
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[0].destination_path = "outside/report.md".into();
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[0].destination_path = "exports\\report.md".into();
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[0].destination_path = "/exports/report.md".into();
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[0].destination_path = "exports/report.md".into();
        grant.destination_dir = "exports/".into();
        assert!(!valid(&grant, &snapshot(&grant)));
    }

    #[test]
    fn only_canonical_relative_destination_paths_are_approvable() {
        let mut grant = record();
        for bad in [
            "exports//report.md",
            "exports/./report.md",
            "exports/../secret.md",
            "exports/report.md/",
            "exports/report\0.md",
        ] {
            grant.files[0].destination_path = bad.into();
            assert!(!valid(&grant, &snapshot(&grant)), "{bad}");
        }

        // Publishing directly to the destination root remains supported.
        grant.destination_dir.clear();
        grant.files[0].destination_path = "report.md".into();
        assert!(valid(&grant, &snapshot(&grant)));
    }

    #[test]
    fn case_collisions_in_outputs_or_destination_paths_fail() {
        let mut grant = record();
        grant.files = vec![
            AgentTaskPublicationPlanFile {
                task_path: "outputs/A.md".into(),
                byte_size: 5,
                sha256: "a".repeat(64),
                destination_path: "exports/A.md".into(),
                action: AgentTaskPublishAction::Create,
            },
            AgentTaskPublicationPlanFile {
                task_path: "outputs/a.md".into(),
                byte_size: 5,
                sha256: "b".repeat(64),
                destination_path: "exports/B.md".into(),
                action: AgentTaskPublishAction::Create,
            },
        ];
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[1].task_path = "outputs/b.md".into();
        grant.files[1].destination_path = "exports/a.MD".into();
        assert!(!valid(&grant, &snapshot(&grant)));

        grant.files[1].destination_path = "exports/B.md".into();
        assert!(valid(&grant, &snapshot(&grant)));
    }

    #[test]
    fn authority_fields_reject_unknown_serialized_inputs() {
        let record = record();
        let mut json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["schemaVersion"], PUBLICATION_APPROVAL_SCHEMA_VERSION);
        assert_eq!(json["destinationRevision"], 42);
        assert_eq!(json["files"][0]["action"], "create");
        assert!(json.get("consumedAtUnixMs").is_none());
        assert!(serde_json::from_value::<PublicationApprovalRecord>(json.clone()).is_ok());
        json["agentApproved"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<PublicationApprovalRecord>(json).is_err());
    }
}
