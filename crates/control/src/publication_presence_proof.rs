//! Opaque result type reserved for future verified native user presence.
//!
//! This is NOT an authenticator or an assertion verifier. There is no
//! production constructor, so a safe Rust caller cannot manufacture proof
//! from an actor string, JSON record, model output or arbitrary digest.
//! A native adapter must verify signed challenge, enrolled key and user
//! verification BEFORE creating this value in a future reviewed change.

/// Opaque, non-cloneable proof for a future qualified native verification.
///
/// The ledger consumes this value by ownership on a claim attempt. A caller
/// cannot reuse the *same safe Rust value* for another grant, even if a claim
/// fails. This does not prevent a compromised verifier from issuing a second
/// proof and is not OS-level isolation or evidence of physical user consent.
///
/// ```compile_fail
/// use infimount_control::publication_presence_proof::VerifiedPublicationPresence;
///
/// fn cannot_replay(proof: VerifiedPublicationPresence) {
///     drop(proof);
///     drop(proof); // use of moved value must fail to compile
/// }
/// ```
pub struct VerifiedPublicationPresence {
    actor_id: String,
    challenge_digest: String,
}

impl VerifiedPublicationPresence {
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    pub fn challenge_digest(&self) -> &str {
        &self.challenge_digest
    }

    /// Test-only substitution for a qualified platform verifier.
    /// Never interpret this fixture as an actual biometric/passkey assertion.
    #[cfg(test)]
    pub(crate) fn test_fixture(actor: &str, digest: &str) -> Self {
        Self {
            actor_id: actor.to_owned(),
            challenge_digest: digest.to_owned(),
        }
    }
}
