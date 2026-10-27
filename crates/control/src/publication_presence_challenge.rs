//! Read-only, domain-separated digest for a FUTURE native user-presence
//! challenge. This module does not issue, verify or consume approvals.
//!
//! In particular, these arguments are not themselves authenticated. A trusted
//! control process must generate fresh nonces and verify a platform-backed
//! assertion bound to this exact digest before any grant can be issued.
use std::collections::HashSet;

use infimount_core::CoreError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::{Uuid, Version};

use crate::publication::{
    normalize_destination_dir, validate_output_task_path, validate_sha256,
    AgentTaskPublishConflictPolicy, MAX_AGENT_TASK_PUBLISH_FILES,
};
use crate::publication_approval::PublicationApprovalSnapshot;
use crate::publication_plan::{AgentTaskPublicationPlanFile, AgentTaskPublishAction};

const CHALLENGE_DOMAIN: &str = "infimount:v0.8.2:native-publication-presence:v1";
const CHALLENGE_SCHEMA_VERSION: u32 = 1;
const MAX_PRESENCE_CHALLENGE_LIFETIME_MS: u64 = 2 * 60 * 1_000;

/// Inputs must be created within the trusted desktop process from a freshly
/// rebuilt, locked publication plan. This is not a model-facing request.
pub struct PresenceChallengeInput<'a> {
    pub control_session_id: &'a str,
    pub challenge_nonce: &'a str,
    pub actor_id: &'a str,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub snapshot: &'a PublicationApprovalSnapshot<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresenceChallengeWire<'a> {
    domain: &'static str,
    schema_version: u32,
    control_session_id: &'a str,
    challenge_nonce: &'a str,
    actor_id: &'a str,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    workspace_id: &'a str,
    task_id: &'a str,
    destination_storage_id: &'a str,
    destination_namespace: &'a str,
    destination_revision: u64,
    destination_dir: &'a str,
    conflict_policy: &'a AgentTaskPublishConflictPolicy,
    preview_fingerprint: &'a str,
    files: &'a [AgentTaskPublicationPlanFile],
}

fn is_random_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .ok()
        .is_some_and(|id| id.get_version() == Some(Version::Random))
}

/// Returns lowercase SHA-256 hexadecimal text representing 32 digest bytes.
/// A future native adapter must decode and bind those bytes using the
/// authenticator's own protocol-defined signed-message format.
/// This is NOT a verified assertion, presence proof or publication capability.
///
/// A caller must additionally prove process/actor authority, fresh trusted
/// randomness, immutable UI presentation and transaction-bound verification.
pub fn presence_challenge_digest(input: &PresenceChallengeInput<'_>) -> Result<String, CoreError> {
    if !is_random_uuid(input.control_session_id)
        || !is_random_uuid(input.challenge_nonce)
        || input.control_session_id == input.challenge_nonce
        || input.actor_id.is_empty()
        || input.actor_id.len() > 128
        || input.actor_id.trim() != input.actor_id
        || input.actor_id.chars().any(char::is_control)
        || input.issued_at_unix_ms >= input.expires_at_unix_ms
        || input.expires_at_unix_ms - input.issued_at_unix_ms > MAX_PRESENCE_CHALLENGE_LIFETIME_MS
    {
        return Err(CoreError::Config(
            "native publication challenge has invalid session, actor or lifetime".into(),
        ));
    }
    let snapshot = input.snapshot;
    if snapshot.conflict_count != 0
        || Uuid::parse_str(snapshot.workspace_id).is_err()
        || Uuid::parse_str(snapshot.task_id).is_err()
        || snapshot.destination_storage_id.trim().is_empty()
        || snapshot.destination_namespace.is_empty()
        || snapshot.destination_revision == 0
        || snapshot.destination_revision == u64::MAX
        || validate_sha256(snapshot.preview_fingerprint).is_err()
        || snapshot.files.is_empty()
        || snapshot.files.len() > MAX_AGENT_TASK_PUBLISH_FILES
        || normalize_destination_dir(snapshot.destination_dir)
            .map_or(true, |canonical| canonical != snapshot.destination_dir)
    {
        return Err(CoreError::Config(
            "native publication challenge requires a conflict-free canonical plan".into(),
        ));
    }
    let mut paths = HashSet::new();
    let mut destinations = HashSet::new();
    for (index, file) in snapshot.files.iter().enumerate() {
        let within_dir = snapshot.destination_dir.is_empty()
            || file
                .destination_path
                .starts_with(&format!("{}/", snapshot.destination_dir));
        if validate_output_task_path(&file.task_path).is_err()
            || file.destination_path.is_empty()
            || validate_sha256(&file.sha256).is_err()
            || file.action == AgentTaskPublishAction::Conflict
            || (snapshot.conflict_policy == &AgentTaskPublishConflictPolicy::Fail
                && file.action != AgentTaskPublishAction::Create)
            || normalize_destination_dir(&file.destination_path)
                .map_or(true, |canonical| canonical != file.destination_path)
            || !within_dir
            || !paths.insert(file.task_path.to_lowercase())
            || !destinations.insert(file.destination_path.to_lowercase())
            || (index > 0 && snapshot.files[index - 1].task_path >= file.task_path)
        {
            return Err(CoreError::Config(
                "native publication challenge contains invalid reviewed file bindings".into(),
            ));
        }
    }
    let wire = PresenceChallengeWire {
        domain: CHALLENGE_DOMAIN,
        schema_version: CHALLENGE_SCHEMA_VERSION,
        control_session_id: input.control_session_id,
        challenge_nonce: input.challenge_nonce,
        actor_id: input.actor_id,
        issued_at_unix_ms: input.issued_at_unix_ms,
        expires_at_unix_ms: input.expires_at_unix_ms,
        workspace_id: snapshot.workspace_id,
        task_id: snapshot.task_id,
        destination_storage_id: snapshot.destination_storage_id,
        destination_namespace: snapshot.destination_namespace,
        destination_revision: snapshot.destination_revision,
        destination_dir: snapshot.destination_dir,
        conflict_policy: snapshot.conflict_policy,
        preview_fingerprint: snapshot.preview_fingerprint,
        files: snapshot.files,
    };
    let canonical_bytes = serde_json::to_vec(&wire)?;
    Ok(format!("{:x}", Sha256::digest(canonical_bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication_approval::{
        PublicationApprovalRecord, PUBLICATION_APPROVAL_SCHEMA_VERSION,
    };

    const SESSION: &str = "6fcfa0a4-e772-4f09-a7de-14c487877798";
    const NONCE: &str = "2fcfaabb-5a8f-46d7-847d-9a53c847b9e5";
    const OTHER_NONCE: &str = "87c990f8-e479-4b32-8a47-7f9a7a3c7890";

    fn record() -> PublicationApprovalRecord {
        PublicationApprovalRecord {
            schema_version: PUBLICATION_APPROVAL_SCHEMA_VERSION,
            grant_id: "421ea05a-95c7-4ef3-a7e3-6b821a0e4ac3".into(),
            actor_id: "verified-person".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
            destination_storage_id: "storage".into(),
            destination_namespace: "trusted-ns".into(),
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
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 61_000,
            nonce: NONCE.into(),
            consumed_at_unix_ms: None,
            revoked_at_unix_ms: None,
        }
    }

    fn make_snapshot(record: &PublicationApprovalRecord) -> PublicationApprovalSnapshot<'_> {
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

    #[test]
    fn challenge_is_deterministic_but_bound_to_nonce_actor_and_policy() {
        let base = record();
        let snapshot = make_snapshot(&base);
        let inputs = PresenceChallengeInput {
            control_session_id: SESSION,
            challenge_nonce: NONCE,
            actor_id: "verified-person",
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 60_000,
            snapshot: &snapshot,
        };
        let digest = presence_challenge_digest(&inputs).unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase()));
        assert_eq!(digest, presence_challenge_digest(&inputs).unwrap());
        let other_nonce = PresenceChallengeInput {
            challenge_nonce: OTHER_NONCE,
            ..inputs
        };
        assert_ne!(digest, presence_challenge_digest(&other_nonce).unwrap());
        let other_actor = PresenceChallengeInput {
            actor_id: "a-different-human",
            ..other_nonce
        };
        assert_ne!(digest, presence_challenge_digest(&other_actor).unwrap());

        let mut changed = make_snapshot(&base);
        changed.destination_revision += 1;
        let changed_input = PresenceChallengeInput {
            snapshot: &changed,
            ..other_actor
        };
        assert_ne!(digest, presence_challenge_digest(&changed_input).unwrap());
    }

    #[test]
    fn challenged_digest_binds_exact_reviewed_source_bytes_and_target() {
        let record = record();
        let original = make_snapshot(&record);
        let request = PresenceChallengeInput {
            control_session_id: SESSION,
            challenge_nonce: NONCE,
            actor_id: "verified-person",
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 60_000,
            snapshot: &original,
        };
        let digest = presence_challenge_digest(&request).unwrap();

        let mut changed = record.clone();
        changed.files[0].sha256 = "c".repeat(64);
        let modified = make_snapshot(&changed);
        let different_source = PresenceChallengeInput {
            snapshot: &modified,
            ..request
        };
        assert_ne!(
            digest,
            presence_challenge_digest(&different_source).unwrap()
        );

        let mut moved_record = record.clone();
        moved_record.files[0].destination_path = "exports/elsewhere.md".into();
        let moved = make_snapshot(&moved_record);
        let other_target = PresenceChallengeInput {
            snapshot: &moved,
            ..different_source
        };
        assert_ne!(digest, presence_challenge_digest(&other_target).unwrap());
    }

    #[test]
    fn empty_or_unsafe_destination_cannot_produce_presence_challenge() {
        for path in [
            "",
            "/report.md",
            "exports//report.md",
            "exports/../secret.md",
            "exports/report.md/",
        ] {
            let mut record = record();
            record.destination_dir.clear();
            record.files[0].destination_path = path.into();
            let current = make_snapshot(&record);
            let challenge = PresenceChallengeInput {
                control_session_id: SESSION,
                challenge_nonce: NONCE,
                actor_id: "verified-person",
                issued_at_unix_ms: 1_000,
                expires_at_unix_ms: 60_000,
                snapshot: &current,
            };
            assert!(presence_challenge_digest(&challenge).is_err(), "{path}");
        }

        let mut record = record();
        record.destination_dir.clear();
        record.files[0].destination_path = "report.md".into();
        let current = make_snapshot(&record);
        let valid = PresenceChallengeInput {
            control_session_id: SESSION,
            challenge_nonce: NONCE,
            actor_id: "verified-person",
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 60_000,
            snapshot: &current,
        };
        assert!(presence_challenge_digest(&valid).is_ok());
    }

    #[test]
    fn malformed_nonce_expiry_or_conflicted_plan_never_produces_digest() {
        let base = record();
        let snapshot = make_snapshot(&base);
        let input = PresenceChallengeInput {
            control_session_id: SESSION,
            challenge_nonce: NONCE,
            actor_id: "verified-person",
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 60_000,
            snapshot: &snapshot,
        };
        assert!(presence_challenge_digest(&input).is_ok());
        let stale = PresenceChallengeInput {
            challenge_nonce: "not-random",
            ..input
        };
        assert!(presence_challenge_digest(&stale).is_err());
        let too_long = PresenceChallengeInput {
            challenge_nonce: NONCE,
            expires_at_unix_ms: 1_000 + MAX_PRESENCE_CHALLENGE_LIFETIME_MS + 1,
            ..stale
        };
        assert!(presence_challenge_digest(&too_long).is_err());

        let mut conflicted_snapshot = make_snapshot(&base);
        conflicted_snapshot.conflict_count = 1;
        let conflicted = PresenceChallengeInput {
            snapshot: &conflicted_snapshot,
            expires_at_unix_ms: 60_000,
            ..too_long
        };
        assert!(presence_challenge_digest(&conflicted).is_err());
    }
}
