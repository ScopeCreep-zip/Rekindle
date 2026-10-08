//! The errors of Rekindle's wire encodings (plan C8): the subset of
//! `rekindle_protocol::CodecError` that encoding and decoding raise,
//! which converts back into it variant by variant.

/// An encoding or decoding failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// A Cap'n Proto union returned a discriminant the local schema
    /// doesn't know about. Distinct from `Deserialization` so callers
    /// can implement gossip-relay forward-compat (verify signature,
    /// decrement TTL, forward bytes intact, do not dispatch).
    #[error("unknown union variant: {0}")]
    UnknownVariant(String),

    #[error("verification failed: {0}")]
    Verification(String),
}
