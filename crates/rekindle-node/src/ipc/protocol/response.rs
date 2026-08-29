//! IPC response and bus payload types.

use serde::{Deserialize, Serialize};

use super::IpcRequest;

// ── IPC Response ────────────────────────────────────────────────────────

/// Daemon → Frontend response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Success with a JSON value payload.
    Ok(serde_json::Value),
    /// Error with code, message, and optional remediation advice.
    Error {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
    /// Unsolicited push: a subscription event from the three-tier event pipeline.
    /// Delivered to all connected clients via the event push task.
    Event(rekindle_types::subscription_events::SubscriptionEvent),
}

impl IpcResponse {
    /// Create a success response from a serializable value.
    pub fn ok<T: Serialize>(value: &T) -> Self {
        // [RC-2] serde_json::to_value can fail on recursive structures,
        // but our types are flat. If it fails, return an internal error.
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

// ── Bus Payload ────────────────────────────────────────────────────────

/// Universal wire payload for the IPC bus.
///
/// Every `Message<BusPayload>` on the bus uses this enum so the server
/// can decode every frame with a single type — required because postcard
/// is schema-aware and cannot decode `Message<IpcRequest>` bytes as
/// `Message<IpcResponse>`.
///
/// The server is a pure router: it decodes `Message<BusPayload>`, routes
/// by `correlation_id` for responses, broadcasts for new requests, and
/// stamps `verified_sender_name`. It never inspects the inner variant.
///
/// Participants wrap their payloads:
/// - CLI/TUI clients: `BusPayload::Request(IpcRequest)`
/// - Daemon subscriber: `BusPayload::Response(IpcResponse)`
/// - Subscription delivery: `BusPayload::Event(SubscriptionEvent)`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusPayload {
    /// Frontend → Daemon request.
    Request(IpcRequest),
    /// Daemon → Frontend response, serialized as JSON bytes.
    ///
    /// `IpcResponse` contains `serde_json::Value` which postcard cannot
    /// handle (postcard requires schema-aware types, not self-describing).
    /// The response is serialized to JSON by the daemon subscriber, carried
    /// as raw bytes through the postcard-encoded bus, and deserialized from
    /// JSON by the client. This is the only type that crosses the postcard/JSON
    /// boundary — all other variants are fully postcard-compatible.
    Response(Vec<u8>),
    /// Daemon → Frontend push event, routed via EventRouter to subscribed connections.
    Event(rekindle_types::subscription_events::SubscriptionEvent),
}
