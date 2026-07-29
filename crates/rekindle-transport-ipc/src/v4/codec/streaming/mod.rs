//! Wire format codecs for v4 streaming frame types.
//!
//! All encode/decode functions operate on fixed or variable-length byte
//! buffers. No heap allocation on the encode path. Decode validates
//! minimum buffer length before reading.

pub mod arena_write;
pub mod slot_release;
pub mod arena_setup;
pub mod arena_ack;
pub mod dmabuf_ref;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { expected, got } => {
                write!(f, "buffer too short: expected {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for CodecError {}
