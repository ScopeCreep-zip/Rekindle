use thiserror::Error;

#[derive(Debug, Error)]
pub enum DmError {
    #[error("invalid public key length: expected 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    #[error("invalid secret key length: expected 32 bytes, got {0}")]
    InvalidSecretKey(usize),
    #[error("hkdf expand failed: {0}")]
    Hkdf(String),
}
