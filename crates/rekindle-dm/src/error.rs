use thiserror::Error;

#[derive(Debug, Error)]
pub enum DmError {
    // --- Key derivation (MEK + slot keys) ---
    #[error("invalid public key length: expected 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    #[error("invalid secret key length: expected 32 bytes, got {0}")]
    InvalidSecretKey(usize),
    #[error("hkdf expand failed: {0}")]
    Hkdf(String),

    // --- Storage (SqliteDmStore) ---
    #[error("storage: {0}")]
    Storage(String),
    #[error("sqlite: {0}")]
    Sqlite(String),
    #[error("serialize: {0}")]
    Serialize(String),

    // --- Session lifecycle ---
    #[error("session not found for peer {0}")]
    SessionNotFound(String),
    #[error("invalid session state: {0}")]
    InvalidSessionState(String),
    #[error("identity not loaded")]
    IdentityNotLoaded,
    #[error("MEK chain unavailable for record {0}")]
    MekChainUnavailable(String),

    // --- Encryption / decryption ---
    #[error("encrypt failed: {0}")]
    EncryptFailed(String),
    #[error("decrypt failed: {0}")]
    DecryptFailed(String),
    #[error("envelope decode failed: {0}")]
    EnvelopeDecode(String),

    // --- Transport (via DmTransportDeps) ---
    #[error("transport: {0}")]
    Transport(String),
    #[error("routing context unavailable")]
    RoutingContextUnavailable,
    #[error("peer route blob unavailable for {0}")]
    PeerRouteUnavailable(String),

    // --- Validation ---
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl DmError {
    /// Convenience constructor for storage-layer errors that arrive as
    /// strings from the rusqlite/tokio-rusqlite boundary.
    pub fn storage<S: Into<String>>(s: S) -> Self {
        DmError::Storage(s.into())
    }

    /// Convenience constructor for transport-layer errors.
    pub fn transport<S: Into<String>>(s: S) -> Self {
        DmError::Transport(s.into())
    }
}
