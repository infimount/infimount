//! Process-private, memory-only single-consumption proof for reviewed
//! publication approvals. This is a **prototype**, not an issuer or an
//! authorization boundary. Native user-presence verification is not wired.
//!
//! There is deliberately no non-test insertion API. A headless process,
//! agent-side MCP tool or serialized approval cannot create grants here.
//! A future trusted native issuer must be reviewed separately and all
//! configuration/workspace mutation paths must invalidate pending grants.
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::Mutex;

use infimount_core::CoreError;

use crate::publication_approval::{
    validate_protected_approval_binding, PublicationApprovalRecord, PublicationApprovalSnapshot,
};
#[cfg(test)]
use crate::publication_presence_challenge::{presence_challenge_digest, PresenceChallengeInput};
use crate::publication_presence_proof::VerifiedPublicationPresence;

struct StoredApproval {
    record: PublicationApprovalRecord,
    /// Private control-session generation at the moment a trusted issuer
    /// would have registered the grant. Never accept this from a client.
    issuance_epoch: u64,
    /// Bound to a verified native challenge by a future trusted issuer,
    /// not to a digest provided through an agent-visible request.
    challenge_digest: String,
}

#[derive(Default)]
struct LedgerState {
    /// In-memory epoch. Config/workspace mutations must invalidate pending
    /// grants before modifying authoritative state, under the same locks.
    epoch: u64,
    sealed: bool,
    records: HashMap<String, StoredApproval>,
    #[cfg(test)]
    used_nonces: HashSet<String>,
}

#[cfg(test)]
fn fixture_challenge_digest(record: &PublicationApprovalRecord) -> Result<String, CoreError> {
    // This is deterministic test data, not a native authenticator, issuer or
    // production nonce generator. The fixtures exercise the *real* versioned
    // challenge serializer instead of accepting a fabricated digest.
    const TEST_PROCESS_ID: &str = "6fcfa0a4-e772-4f09-a7de-14c487877798";
    let snapshot = PublicationApprovalSnapshot {
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
    };
    presence_challenge_digest(&PresenceChallengeInput {
        control_session_id: TEST_PROCESS_ID,
        challenge_nonce: &record.nonce,
        actor_id: &record.actor_id,
        issued_at_unix_ms: record.issued_at_unix_ms,
        expires_at_unix_ms: record.expires_at_unix_ms,
        snapshot: &snapshot,
    })
}

/// Fresh on every trusted service boot. Never serialize or back up this state.
/// A second independent instance does not inherit authority from the first.
#[derive(Default)]
pub struct EphemeralPublicationApprovalLedger {
    state: Mutex<LedgerState>,
}

impl EphemeralPublicationApprovalLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate a server-held grant against a freshly rebuilt, trusted plan
    /// and consume it while holding the ledger mutex, BEFORE publication I/O.
    ///
    /// Callers must already hold lifecycle → configuration → workspace locks.
    /// The opaque proof has NO production constructor until a native
    /// platform verifier is implemented and separately qualified.
    pub fn claim(
        &self,
        grant_id: &str,
        snapshot: &PublicationApprovalSnapshot<'_>,
        verified_presence: &VerifiedPublicationPresence,
        now_unix_ms: u64,
    ) -> Result<(), CoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoreError::Config("publication approval ledger is unavailable".into()))?;
        let epoch = state.epoch;
        let sealed = state.sealed;
        let stored = state.records.get_mut(grant_id).ok_or_else(|| {
            CoreError::Config("publication approval was not issued by this process".into())
        })?;
        // Even if restored storage bytes and its persisted revision return to
        // an earlier value (ABA), a grant from the prior process epoch cannot
        // be claimed. This remains only a prototype without a trusted issuer.
        if sealed
            || stored.issuance_epoch != epoch
            || stored.challenge_digest != verified_presence.challenge_digest()
        {
            stored.record.revoked_at_unix_ms = Some(now_unix_ms);
            return Err(CoreError::Config(
                "publication approval belongs to a stale session or different challenge".into(),
            ));
        }

        // A mismatching attempt revokes the grant rather than permitting a
        // later replay after a mutation, expiry or actor substitution.
        if let Err(error) = validate_protected_approval_binding(
            &stored.record,
            snapshot,
            verified_presence.actor_id(),
            now_unix_ms,
        ) {
            stored.record.revoked_at_unix_ms = Some(now_unix_ms);
            return Err(error);
        }

        stored.record.consumed_at_unix_ms = Some(now_unix_ms);
        Ok(())
    }

    /// Future mutation handlers call this under their authoritative locks
    /// *before* changing any storage/workspace configuration. Keep consumed
    /// grants and nonce tombstones to reject replay until process exit.
    pub fn invalidate_pending(&self, now_unix_ms: u64) -> Result<(), CoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoreError::Config("publication approval ledger is unavailable".into()))?;
        // Never wrap or saturate a grant epoch. Exhaustion permanently seals
        // this in-memory ledger, including future attempted test insertions.
        match state.epoch.checked_add(1) {
            Some(next) if !state.sealed => state.epoch = next,
            _ => state.sealed = true,
        }
        for stored in state.records.values_mut() {
            if stored.record.consumed_at_unix_ms.is_none()
                && stored.record.revoked_at_unix_ms.is_none()
            {
                stored.record.revoked_at_unix_ms = Some(now_unix_ms);
            }
        }
        if state.sealed {
            return Err(CoreError::Config(
                "publication approval session epoch is exhausted".into(),
            ));
        }
        Ok(())
    }

    /// Prototype insertion exists only inside this crate's tests. No product
    /// code, CLI, IPC or MCP entry point can mint an approval.
    #[cfg(test)]
    fn insert_fixture(&self, record: PublicationApprovalRecord) -> Result<(), CoreError> {
        let challenge_digest = fixture_challenge_digest(&record)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoreError::Config("publication approval ledger is unavailable".into()))?;
        if state.sealed
            || state.records.contains_key(&record.grant_id)
            || !state.used_nonces.insert(record.nonce.clone())
        {
            return Err(CoreError::Config(
                "publication grant epoch, id or nonce is invalid or already registered".into(),
            ));
        }
        let epoch = state.epoch;
        state.records.insert(
            record.grant_id.clone(),
            StoredApproval {
                record,
                issuance_epoch: epoch,
                challenge_digest,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::AgentTaskPublishConflictPolicy;
    use crate::publication_plan::{AgentTaskPublicationPlanFile, AgentTaskPublishAction};

    fn grant() -> PublicationApprovalRecord {
        PublicationApprovalRecord {
            schema_version: 1,
            grant_id: "421ea05a-95c7-4ef3-a7e3-6b821a0e4ac3".into(),
            actor_id: "native-verified-human".into(),
            workspace_id: "f8f47aa7-702d-4fd6-8815-84cbdf3b3127".into(),
            task_id: "81f08176-86e4-40ec-a9a4-a219c4c9b454".into(),
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
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 61_000,
            nonce: "c7da7279-1fc9-4f7f-ad1e-114706628bf1".into(),
            consumed_at_unix_ms: None,
            revoked_at_unix_ms: None,
        }
    }

    fn verified_actor(
        record: &PublicationApprovalRecord,
        actor: &str,
    ) -> VerifiedPublicationPresence {
        VerifiedPublicationPresence::test_fixture(actor, &fixture_challenge_digest(record).unwrap())
    }

    fn snapshot(record: &PublicationApprovalRecord) -> PublicationApprovalSnapshot<'_> {
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
    fn one_process_can_claim_exactly_once_even_under_concurrency() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        let grant_id = record.grant_id.as_str();

        let claims = std::thread::scope(|scope| {
            let handles = (0..16)
                .map(|_| {
                    let ledger = &ledger;
                    let current = &current;
                    let record = &record;
                    scope.spawn(move || {
                        ledger
                            .claim(
                                grant_id,
                                current,
                                &verified_actor(record, "native-verified-human"),
                                2_000,
                            )
                            .is_ok()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .filter(|success| *success)
                .count()
        });
        assert_eq!(claims, 1);
    }

    #[test]
    fn matching_actor_cannot_replay_different_verified_challenge() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let grant = grant();
        ledger.insert_fixture(grant.clone()).unwrap();
        let current = snapshot(&grant);
        let incorrect =
            VerifiedPublicationPresence::test_fixture("native-verified-human", &"b".repeat(64));
        let rejected = ledger.claim(&grant.grant_id, &current, &incorrect, 2_000);
        assert!(rejected.is_err());
        // A mismatching proof spends the chance to claim this grant.
        let valid = verified_actor(&grant, "native-verified-human");
        let replay = ledger.claim(&grant.grant_id, &current, &valid, 2_010);
        assert!(replay.is_err());
    }

    #[test]
    fn failed_publication_does_not_reset_spent_grant() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        ledger
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                2_000,
            )
            .unwrap();

        // Simulate output/receipt failure after consumption. The grant stays
        // spent regardless of any subsequent cleanup or retry.
        assert!(ledger
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                2_100,
            )
            .is_err());
    }

    #[test]
    fn mutation_invalidates_pending_and_boot_restart_drops_all_grants() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        ledger.invalidate_pending(2_000).unwrap();
        assert!(ledger
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                2_100,
            )
            .is_err());

        let restarted = EphemeralPublicationApprovalLedger::new();
        assert!(restarted
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                2_100,
            )
            .is_err());
    }

    #[test]
    fn restore_aba_cannot_revalidate_a_grant_from_an_earlier_session_epoch() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let old = grant();
        ledger.insert_fixture(old.clone()).unwrap();
        let same_snapshot = snapshot(&old);

        // Simulates storage and workspace being changed, backed up, and then
        // restored byte-for-byte with the SAME recorded revision and plan.
        ledger.invalidate_pending(2_000).unwrap();
        // Stronger than a revocation-only test: even if a future recovery
        // regression accidentally resurrected the old pending record,
        // the private epoch mismatch must independently deny its claim.
        {
            let mut state = ledger.state.lock().unwrap();
            let stale = state.records.get_mut(&old.grant_id).unwrap();
            stale.record.revoked_at_unix_ms = None;
        }
        let old_claim = ledger.claim(
            &old.grant_id,
            &same_snapshot,
            &verified_actor(&old, "native-verified-human"),
            2_010,
        );
        assert!(old_claim.is_err());

        // A fresh fixture with a distinct nonce can be registered only by
        // the test-only issuer. The new epoch does not permanently block
        // legitimate later approvals while rejecting the old one.
        let mut fresh = old.clone();
        fresh.grant_id = "81d4c2aa-9a35-44fd-9b79-3b8b5e28c765".into();
        fresh.nonce = "911ab9da-edce-41b8-9c23-aceb3ee5d643".into();
        ledger.insert_fixture(fresh.clone()).unwrap();
        let current = snapshot(&fresh);
        ledger
            .claim(
                &fresh.grant_id,
                &current,
                &verified_actor(&fresh, "native-verified-human"),
                2_020,
            )
            .unwrap();
        let replay = ledger.claim(
            &old.grant_id,
            &same_snapshot,
            &verified_actor(&old, "native-verified-human"),
            2_030,
        );
        assert!(replay.is_err());
    }

    #[test]
    fn epoch_overflow_permanently_seals_grant_claim_and_issuance() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let old = grant();
        ledger.insert_fixture(old.clone()).unwrap();
        {
            let mut state = ledger.state.lock().unwrap();
            state.epoch = u64::MAX;
        }
        assert!(ledger.invalidate_pending(2_000).is_err());
        let present = snapshot(&old);
        let overflow_claim = ledger.claim(
            &old.grant_id,
            &present,
            &verified_actor(&old, "native-verified-human"),
            2_010,
        );
        assert!(overflow_claim.is_err());

        let mut fresh = old;
        fresh.grant_id = "20563794-cde0-46db-9ef9-d056d9f51c7b".into();
        fresh.nonce = "91ed31f9-026c-438c-892f-4b3b42e45e3c".into();
        assert!(ledger.insert_fixture(fresh).is_err());
        assert!(ledger.invalidate_pending(2_020).is_err());
    }

    #[test]
    fn stale_plan_wrong_actor_expired_grant_or_nonce_replay_fail_closed() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        assert!(ledger
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "model-process"),
                2_000,
            )
            .is_err());
        assert!(ledger
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                2_100,
            )
            .is_err());

        let second = EphemeralPublicationApprovalLedger::new();
        second.insert_fixture(record.clone()).unwrap();
        let mut stale = snapshot(&record);
        stale.destination_revision += 1;
        assert!(second
            .claim(
                &record.grant_id,
                &stale,
                &verified_actor(&record, "native-verified-human"),
                2_000,
            )
            .is_err());

        let third = EphemeralPublicationApprovalLedger::new();
        third.insert_fixture(record.clone()).unwrap();
        assert!(third
            .claim(
                &record.grant_id,
                &current,
                &verified_actor(&record, "native-verified-human"),
                61_000,
            )
            .is_err());

        let mut another = record.clone();
        another.grant_id = "89846fbd-c253-42ff-9ab7-aed9d2b0c200".into();
        assert!(third.insert_fixture(another).is_err());
    }

    #[test]
    fn genuine_challenge_digest_rejects_a_copied_actor_and_modified_plan() {
        let record = grant();
        let digest = fixture_challenge_digest(&record).unwrap();
        assert_ne!(digest, "a".repeat(64));
        let snapshot = snapshot(&record);

        // A proof for a new nonce is not equivalent to human approval of
        // the pending grant, even when the actor and reviewed paths match.
        let mut other_nonce = record.clone();
        other_nonce.nonce = "911ab9da-edce-41b8-9c23-aceb3ee5d643".into();
        let wrong_nonce = verified_actor(&other_nonce, &record.actor_id);
        let ledger = EphemeralPublicationApprovalLedger::new();
        ledger.insert_fixture(record.clone()).unwrap();
        assert!(ledger
            .claim(&record.grant_id, &snapshot, &wrong_nonce, 2_000)
            .is_err());
        assert!(ledger
            .claim(
                &record.grant_id,
                &snapshot,
                &verified_actor(&record, &record.actor_id),
                2_010,
            )
            .is_err());

        // A changed file digest is rejected even when its byte size,
        // destination, actor, storage revision and preview are unchanged.
        let mut other_bytes = record.clone();
        other_bytes.files[0].sha256 = "c".repeat(64);
        let stale_proof = verified_actor(&other_bytes, &record.actor_id);
        let another = EphemeralPublicationApprovalLedger::new();
        another.insert_fixture(record.clone()).unwrap();
        assert!(another
            .claim(&record.grant_id, &snapshot, &stale_proof, 2_000)
            .is_err());
    }
}
