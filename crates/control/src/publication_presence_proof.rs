//! Opaque result type reserved for future verified native user presence.
//!
//! This is NOT an authenticator or an assertion verifier. There is no
//! production constructor, so a safe Rust caller cannot manufacture proof
//! from an actor string, JSON record, model output or arbitrary digest.
//! A native adapter must verify signed challenge, enrolled key and user
//! verification BEFORE creating this value in a future reviewed change.

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
