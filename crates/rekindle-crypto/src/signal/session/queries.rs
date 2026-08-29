//! Read-only session queries: existence, TOFU identity trust, deletion.

use crate::error::CryptoError;

use super::SignalSessionManager;

impl SignalSessionManager {
    /// Check if we have an established session with a peer.
    pub fn has_session(&self, peer_address: &str) -> Result<bool, CryptoError> {
        self.sessions.has_session(peer_address)
    }

    /// TOFU check: whether the given `identity_key` matches the trusted
    /// record for `peer_address`. Returns `Ok(true)` for a first-contact
    /// peer (no stored record) — that's the trust-on-first-use semantics
    /// per `IdentityKeyStore.java:54-60` (libsignal). Returns `Ok(false)`
    /// only when there IS a stored record AND the keys disagree —
    /// signaling that the peer's identity rotated (legitimate re-onboard
    /// or substitution attack; either way the application layer should
    /// require explicit user consent before wiping the existing session).
    ///
    /// W16.10d follow-up — exposes the underlying `IdentityKeyStore`
    /// trait method so callers don't have to thread the store separately.
    pub fn is_trusted_identity(
        &self,
        peer_address: &str,
        identity_key: &[u8],
    ) -> Result<bool, CryptoError> {
        self.identity
            .is_trusted_identity(peer_address, identity_key)
    }

    /// Delete an existing session with a peer (e.g., on friend removal).
    pub fn delete_session(&self, peer_address: &str) -> Result<(), CryptoError> {
        self.sessions.delete_session(peer_address)
    }
}
