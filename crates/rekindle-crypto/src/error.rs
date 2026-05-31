use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key generation failed: {0}")]
    KeyGeneration(String),

    #[error("signing failed: {0}")]
    SigningError(String),

    #[error("verification failed: {0}")]
    VerificationError(String),

    #[error("encryption failed: {0}")]
    EncryptionError(String),

    #[error("decryption failed: {0}")]
    DecryptionError(String),

    #[error("invalid key material: {0}")]
    InvalidKey(String),

    #[error("signal session error: {0}")]
    SessionError(String),

    #[error("prekey error: {0}")]
    PreKeyError(String),

    #[error("key storage error: {0}")]
    StorageError(String),

    /// Phase 6 — no session exists for the requested peer (neither in
    /// the in-memory cache nor in persistent storage).
    #[error("no session for peer {0}")]
    NoSession(String),

    /// Phase 6 — vault is locked (passphrase not yet entered); persistent
    /// load/store of session state is unavailable until unlock.
    #[error("vault locked — session persistence unavailable")]
    VaultLocked,
}
