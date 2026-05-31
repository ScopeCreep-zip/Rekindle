//! Errors for Phase 18 community lifecycle ops.
//!
//! Each variant maps to a failure mode in apply / origin / bootstrap /
//! join / segments. The `Adapter` variant wraps src-tauri adapter
//! failures (Veilid attach loss, Stronghold I/O, SQL failure) as opaque
//! strings — the crate never types these directly.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GovernanceRuntimeError {
    #[error("community not found: {0}")]
    CommunityNotFound(String),

    #[error("governance state not loaded for community {0}")]
    GovernanceStateMissing(String),

    #[error("identity secret not available")]
    IdentitySecretUnavailable,

    #[error("not attached to Veilid")]
    NotAttached,

    #[error("no pseudonym key for community {0}")]
    PseudonymKeyMissing(String),

    #[error("invalid pseudonym hex: {0}")]
    InvalidPseudonymHex(String),

    #[error("no governance key for community {0}")]
    GovernanceKeyMissing(String),

    #[error("no slot index for community {0}")]
    SlotIndexMissing(String),

    #[error("no slot keypair for community {0}")]
    SlotKeypairMissing(String),

    #[error("no slot seed for community {0}")]
    SlotSeedMissing(String),

    #[error("insufficient permission for this governance operation")]
    PermissionDenied,

    #[error("governance write conflicted with newer network state ({0} bytes)")]
    WriteConflict(usize),

    #[error("governance verify failed: read-back differs ({read} bytes vs our {written})")]
    VerifyMismatch { read: usize, written: usize },

    #[error("governance verify read-back returned empty after write")]
    VerifyEmpty,

    #[error("segment cap reached ({0}); raise MAX_SEGMENTS once lazy-fetch lands")]
    SegmentCapReached(u32),

    #[error("current segment still has open slots — expansion is only allowed when full")]
    SegmentNotFull,

    #[error("crypto/serialization error: {0}")]
    Crypto(String),

    #[error("encoding error: {0}")]
    Encoding(String),

    #[error("adapter error: {0}")]
    Adapter(String),
}

impl GovernanceRuntimeError {
    /// Convenience constructor for src-tauri adapter impls so they can
    /// surface tauri/veilid/sqlite errors through one variant without
    /// the crate having to know about those types.
    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }
}
