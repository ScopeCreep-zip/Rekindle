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
}
