//! Daemon response types — what the daemon returns for every request.
//!
//! Pure Ok/Error. No Event variant — events flow through the transport's
//! pub/sub mechanism (`DATAGRAM_PUBLISH`), not inside responses.

use serde::{Deserialize, Serialize};

/// Daemon → Frontend response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Success with a JSON value payload.
    Ok(serde_json::Value),
    /// Error with code, message, and optional remediation advice.
    Error {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
}

impl DaemonResponse {
    /// Create a success response from a serializable value.
    pub fn ok<T: Serialize>(value: &T) -> Self {
        match serde_json::to_value(value) {
            std::result::Result::Ok(v) => Self::Ok(v),
            Err(e) => Self::Error {
                code: 500,
                message: format!("response serialization failed: {e}"),
                remediation: None,
            },
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
