//! Phase 16 — VideoError for crate-side fallible operations.
//!
//! The existing `FragmentError` and `ReassemblerError` cover wire-level
//! framing. `VideoError` is the broader error type returned by
//! `VideoDeps`-parameterised fns (send/receive orchestration), wrapping
//! the framing errors plus the new send/transport/encrypt variants.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("identity not unlocked — cannot derive signing key")]
    IdentityNotLoaded,

    #[error("MEK unavailable for community {community} — join voice/video first")]
    MekUnavailable { community: String },

    #[error("encrypt failed: {0}")]
    Encrypt(String),

    #[error("decrypt failed: {0}")]
    Decrypt(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("fragment: {0}")]
    Fragment(#[from] crate::fragment::FragmentError),
}
