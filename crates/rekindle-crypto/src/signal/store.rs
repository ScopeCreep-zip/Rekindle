use crate::CryptoError;

/// Storage trait for Signal Protocol identity keys.
///
/// Maps Ed25519 public keys to trusted/untrusted status.
/// Implemented by the Tauri backend using Stronghold + `SQLite`.
pub trait IdentityKeyStore: Send + Sync {
    /// Get our own identity key pair (Ed25519 private + public).
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>), CryptoError>;

    /// Get our local registration ID.
    fn get_local_registration_id(&self) -> Result<u32, CryptoError>;

    /// Check if a remote identity key is trusted.
    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool, CryptoError>;

    /// Save a remote identity key (TOFU — Trust On First Use).
    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError>;
}

/// Classifier for PQXDH ML-KEM prekeys — distinguishes the long-rotation
/// last-resort key from one-time keys that are consumed on first use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PqKeyKind {
    /// Long-rotation ML-KEM-768 key. One per identity at a time. Never
    /// consumed; rotates rarely.
    LastResort,
    /// One-time ML-KEM-768 key. Consumed by the responder when the
    /// initiator's encapsulation targeted it.
    OneTime,
}

/// Storage trait for Signal Protocol prekeys.
///
/// Classical X3DH one-time prekeys are consumed after first use.
/// Phase 3b of the decomposed-harvest plan added PQXDH ML-KEM-768
/// prekeys via the `store_pq_secret` / `load_pq_secret` / `remove_pq_secret`
/// methods.
pub trait PreKeyStore: Send + Sync {
    /// Load a classical one-time prekey by ID.
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Store a classical one-time prekey.
    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError>;

    /// Remove a consumed classical one-time prekey.
    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError>;

    /// Load the current classical signed prekey.
    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Store a classical signed prekey.
    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError>;

    /// Phase 3b — load an ML-KEM-768 secret by `(id, kind)`. Returns
    /// `Ok(None)` if the key isn't in the store (e.g. consumed one-time).
    fn load_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
    ) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Phase 3b — store an ML-KEM-768 secret under `(id, kind)`. The
    /// `key_data` is the 2400-byte FIPS-203 secret blob.
    fn store_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
        key_data: &[u8],
    ) -> Result<(), CryptoError>;

    /// Phase 3b — remove a consumed PQ one-time prekey. No-op for
    /// `LastResort` (which is rotated, not consumed).
    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<(), CryptoError>;
}

/// Storage trait for Signal Protocol sessions.
///
/// Each peer has at most one active session. Sessions are keyed by
/// the peer's Ed25519 public key (hex-encoded).
pub trait SessionStore: Send + Sync {
    /// Load session state for a peer.
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Store session state for a peer.
    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError>;

    /// Check if a session exists for a peer.
    fn has_session(&self, address: &str) -> Result<bool, CryptoError>;

    /// Delete a session (e.g., on friend removal).
    fn delete_session(&self, address: &str) -> Result<(), CryptoError>;

    /// List all sessions (for session management UI).
    fn list_sessions(&self) -> Result<Vec<String>, CryptoError>;
}
