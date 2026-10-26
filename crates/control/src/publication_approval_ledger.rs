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

#[derive(Default)]
struct LedgerState {
    records: HashMap<String, PublicationApprovalRecord>,
    #[cfg(test)]
    used_nonces: HashSet<String>,
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
    /// Callers must already hold lifecycle → configuration → workspace locks
    /// and establish the actor through a future native trust ceremony. The
    /// supplied actor string is not authenticated by this function itself.
    pub fn claim(
        &self,
        grant_id: &str,
        snapshot: &PublicationApprovalSnapshot<'_>,
        authenticated_actor_id: &str,
        now_unix_ms: u64,
    ) -> Result<(), CoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoreError::Config("publication approval ledger is unavailable".into()))?;
        let record = state.records.get_mut(grant_id).ok_or_else(|| {
            CoreError::Config("publication approval was not issued by this process".into())
        })?;

        // A mismatching attempt revokes the grant rather than permitting a
        // later replay after a mutation, expiry or actor substitution.
        if let Err(error) = validate_protected_approval_binding(
            record,
            snapshot,
            authenticated_actor_id,
            now_unix_ms,
        ) {
            record.revoked_at_unix_ms = Some(now_unix_ms);
            return Err(error);
        }

        record.consumed_at_unix_ms = Some(now_unix_ms);
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
        for record in state.records.values_mut() {
            if record.consumed_at_unix_ms.is_none() && record.revoked_at_unix_ms.is_none() {
                record.revoked_at_unix_ms = Some(now_unix_ms);
            }
        }
        Ok(())
    }

    /// Prototype insertion exists only inside this crate's tests. No product
    /// code, CLI, IPC or MCP entry point can mint an approval.
    #[cfg(test)]
    fn insert_fixture(&self, record: PublicationApprovalRecord) -> Result<(), CoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoreError::Config("publication approval ledger is unavailable".into()))?;
        if state.records.contains_key(&record.grant_id)
            || !state.used_nonces.insert(record.nonce.clone())
        {
            return Err(CoreError::Config(
                "publication grant id or one-use nonce is already registered".into(),
            ));
        }
        state.records.insert(record.grant_id.clone(), record);
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
                    scope.spawn(move || {
                        ledger
                            .claim(grant_id, current, "native-verified-human", 2_000)
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
    fn failed_publication_does_not_reset_spent_grant() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        ledger
            .claim(&record.grant_id, &current, "native-verified-human", 2_000)
            .unwrap();

        // Simulate output/receipt failure after consumption. The grant stays
        // spent regardless of any subsequent cleanup or retry.
        assert!(ledger
            .claim(&record.grant_id, &current, "native-verified-human", 2_100)
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
            .claim(&record.grant_id, &current, "native-verified-human", 2_100)
            .is_err());

        let restarted = EphemeralPublicationApprovalLedger::new();
        assert!(restarted
            .claim(&record.grant_id, &current, "native-verified-human", 2_100)
            .is_err());
    }

    #[test]
    fn stale_plan_wrong_actor_expired_grant_or_nonce_replay_fail_closed() {
        let ledger = EphemeralPublicationApprovalLedger::new();
        let record = grant();
        ledger.insert_fixture(record.clone()).unwrap();
        let current = snapshot(&record);
        assert!(ledger
            .claim(&record.grant_id, &current, "model-process", 2_000)
            .is_err());
        assert!(ledger
            .claim(&record.grant_id, &current, "native-verified-human", 2_100)
            .is_err());

        let second = EphemeralPublicationApprovalLedger::new();
        second.insert_fixture(record.clone()).unwrap();
        let mut stale = snapshot(&record);
        stale.destination_revision += 1;
        assert!(second
            .claim(&record.grant_id, &stale, "native-verified-human", 2_000)
            .is_err());

        let third = EphemeralPublicationApprovalLedger::new();
        third.insert_fixture(record.clone()).unwrap();
        assert!(third
            .claim(&record.grant_id, &current, "native-verified-human", 61_000)
            .is_err());

        let mut another = record.clone();
        another.grant_id = "89846fbd-c253-42ff-9ab7-aed9d2b0c200".into();
        assert!(third.insert_fixture(another).is_err());
    }
}
