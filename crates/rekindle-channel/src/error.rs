//! Errors for Phase 19 channel-messaging ops.
//!
//! Each variant maps to a failure mode in send / receive / threads /
//! reactions / expressions / mentions. The `Adapter` variant wraps
//! src-tauri adapter failures (Veilid attach loss, SQLite failure,
//! signing errors) as opaque strings — the crate never types these
//! directly.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    /// The community's Lamport clock could not produce a timestamp.
    #[error(transparent)]
    Lamport(#[from] rekindle_types::lamport::LamportError),

    #[error("community not found: {0}")]
    CommunityNotFound(String),

    #[error("channel not found: {0}")]
    ChannelNotFound(String),

    #[error("identity secret not available")]
    IdentitySecretUnavailable,

    #[error("no pseudonym key for community {0}")]
    PseudonymKeyMissing(String),

    #[error("no MEK available for channel {channel}/community {community}")]
    MekMissing { community: String, channel: String },

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// We were removed from the community (plan E3.2 S9 own removal): it is
    /// read-only history; rejoining takes a new invite.
    #[error("you were {} this community", if *banned { "banned from" } else { "removed from" })]
    Removed { banned: bool },

    #[error("slowmode active — wait {wait_ms}ms before sending again")]
    SlowmodeActive { wait_ms: u64 },

    #[error("message body exceeds max size ({size} > {max})")]
    BodyTooLarge { size: usize, max: usize },

    #[error("invalid invite/thread/expression id: {0}")]
    InvalidId(String),

    #[error("encrypt failed: {0}")]
    Encrypt(String),

    #[error("decrypt failed: {0}")]
    Decrypt(String),

    #[error("encoding error: {0}")]
    Encoding(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("adapter error: {0}")]
    Adapter(String),

    /// An edit or a delete the rules refuse (plan E3.3 E1–E3).
    #[error("message change refused: {0}")]
    ChangeRefused(crate::changes::ChangeRefusal),
}

impl ChannelError {
    /// Convenience constructor for src-tauri adapter impls so they can
    /// surface tauri/veilid/sqlite errors through one variant without
    /// the crate having to know about those types.
    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }
}
