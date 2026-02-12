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

/// Storage trait for Signal Protocol prekeys.
///
/// One-time prekeys are consumed after first use.
pub trait PreKeyStore: Send + Sync {
    /// Load a prekey by ID.
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Store a prekey.
    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError>;

    /// Remove a consumed prekey.
    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError>;

    /// Load the current signed prekey.
    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Store a signed prekey.
    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError>;
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
