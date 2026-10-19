//! PQXDH (Signal's PQXDH rev. 3): the key agreement that opens a session
//! (`agreement`), with the key encodings its KDF and AD bind (`encode`),
//! the key parsing (`keys`) and the bundle signature checks (`verify`).
//! `SessionRecord` (`super::record`) runs it.

pub mod agreement;
pub mod encode;
pub mod keys;
pub mod verify;
#[derive(Debug, thiserror::Error)]
pub enum PqxdhError {
    #[error("invalid X25519 key bytes (expected 32, got {0})")]
    InvalidX25519Length(usize),
    #[error("Ed25519 → X25519 conversion failed: {0}")]
    EdToX25519(String),
    #[error("invalid ML-KEM public key (expected 1184 bytes)")]
    InvalidMlKemPublic,
    #[error("invalid ML-KEM ciphertext (expected 1088 bytes)")]
    InvalidMlKemCiphertext,
    #[error("Ed25519 signature verification failed: {0}")]
    SignatureVerify(String),
    #[error("HKDF expand failed: {0}")]
    Hkdf(String),
    #[error("PQ prekey bundle missing both one-time and last-resort PQ keys")]
    NoPqKey,
    #[error("invalid Ed25519 identity key: {0}")]
    InvalidIdentityKey(String),
    /// A DH output was all zeros: the peer's key is a low-order point.
    #[error("non-contributory X25519 agreement")]
    NonContributory,
    /// A one-time key arrived without its id or signature, or the reverse.
    #[error("incomplete prekey bundle: {0}")]
    IncompleteBundle(&'static str),
}
