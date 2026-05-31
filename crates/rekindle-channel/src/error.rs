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
}

impl ChannelError {
    /// Convenience constructor for src-tauri adapter impls so they can
    /// surface tauri/veilid/sqlite errors through one variant without
    /// the crate having to know about those types.
    pub fn adapter(msg: impl Into<String>) -> Self {
        Self::Adapter(msg.into())
    }
}
