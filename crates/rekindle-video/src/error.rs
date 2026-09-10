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

    /// A [`codec::VideoEncoder`](crate::codec::VideoEncoder) impl failed
    /// while encoding a frame (e.g. libvpx returned a non-OK status).
    #[error("encode failed: {0}")]
    Encode(String),

    /// [`encode`](crate::codec::VideoEncoder::encode) was called before
    /// [`configure`](crate::codec::VideoEncoder::configure).
    #[error("encoder not configured — call configure() first")]
    EncoderNotConfigured,

    /// The requested encoder or codec is not available in this build —
    /// e.g. `rekindle-video-libvpx` compiled without its `libvpx`
    /// feature, or a codec (H.264) the selected engine cannot encode.
    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("fragment: {0}")]
    Fragment(#[from] crate::fragment::FragmentError),
}
