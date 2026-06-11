//! Daemon response types — what the daemon returns for every request.
//!
//! Pure Ok/Error. No Event variant — events flow through the transport's
//! pub/sub mechanism (`DATAGRAM_PUBLISH`), not inside responses.
//!
//! Wire format: postcard for the envelope, JSON for the Ok payload.
//! `Ok(Vec<u8>)` carries pre-serialized JSON bytes. The consumer calls
//! `serde_json::from_slice()` on the inner bytes. This two-layer approach
//! keeps postcard as the single envelope codec (no `deserialize_any`
//! constraint) while allowing arbitrary JSON payloads.

use serde::{Deserialize, Serialize};

/// Daemon → Frontend response.
///
/// `Ok` carries pre-serialized JSON bytes, not `serde_json::Value`.
/// This allows postcard as the envelope codec — postcard cannot
/// round-trip `serde_json::Value` because `Value::deserialize` requires
/// a self-describing format.
///
/// Construct via `DaemonResponse::ok(&value)` which JSON-serializes
/// the value into `Vec<u8>`. Consume via `DaemonResponse::parse_ok()`
/// or `serde_json::from_slice()` on the inner bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Success with a pre-serialized JSON payload.
    Ok(Vec<u8>),
    /// Error with code, message, and optional remediation advice.
    Error {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
}

impl DaemonResponse {
    /// Serialize the envelope to bytes via postcard.
    ///
    /// This is the SSOT for the wire format. `routing.rs` (server) and
    /// `rekindle-client` (client) both call this — neither chooses the codec.
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserialize the envelope from bytes via postcard.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }

    /// Create a success response from a serializable value.
    ///
    /// JSON-serializes `value` into `Vec<u8>`. The consumer deserializes
    /// the inner bytes with `serde_json::from_slice()` or `parse_ok()`.
    pub fn ok<T: Serialize>(value: &T) -> Self {
        match serde_json::to_vec(value) {
            std::result::Result::Ok(bytes) => Self::Ok(bytes),
            Err(e) => Self::Error {
                code: 500,
                message: format!("response serialization failed: {e}"),
                remediation: None,
            },
        }
    }

    /// Parse the Ok payload as a `serde_json::Value`.
    ///
    /// Convenience method for consumers that want `Value`. Returns
    /// `None` for `Error` variants, `Some(Err)` for invalid JSON.
    pub fn parse_ok(&self) -> Option<Result<serde_json::Value, serde_json::Error>> {
        match self {
            Self::Ok(bytes) => Some(serde_json::from_slice(bytes)),
            Self::Error { .. } => None,
        }
    }

    /// Create an error response.
    #[must_use]
    pub fn error(code: u32, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
            remediation: None,
        }
    }

    /// Create an error response with remediation advice.
    #[must_use]
    pub fn error_with_remediation(
        code: u32,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self::Error {
            code,
            message: message.into(),
            remediation: Some(remediation.into()),
        }
    }
}

/// Context for marking a conversation as read.
///
/// Carried in [`DaemonRequest::MarkRead`](super::request::DaemonRequest::MarkRead).
/// Makes invalid states unrepresentable — exactly one of Channel or Dm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadContext {
    Channel { community: String, channel: String },
    Dm { peer: String },
}
