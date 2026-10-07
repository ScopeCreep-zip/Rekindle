//! Phase 17 — MEK rotation error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MekRotationError {
    /// The community's Lamport clock could not produce a timestamp.
    #[error(transparent)]
    Lamport(#[from] rekindle_types::lamport::LamportError),

    #[error("cache: {0}")]
    Cache(String),

    #[error("persist: {0}")]
    Persist(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("crypto: {0}")]
    Crypto(String),

    #[error("no eligible rotator — all online members exhausted cascade attempts")]
    NoEligibleRotator,

    /// This member holds none of the rotation permissions (KICK, BAN,
    /// MANAGE_COMMUNITY), so it may not rotate the community key.
    #[error("not permitted to rotate the community key")]
    NotPermitted,

    #[error("generation mismatch: expected {expected}, got {actual}")]
    GenerationMismatch { expected: u64, actual: u64 },

    #[error("no MEK cached for community {0}")]
    NoMek(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("pseudonym missing for community {0}")]
    PseudonymMissing(String),

    #[error("identity not unlocked")]
    IdentityNotLoaded,
}
