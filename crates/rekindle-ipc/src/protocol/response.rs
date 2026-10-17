//! IPC response and bus payload types.

use serde::{Deserialize, Serialize};

use super::IpcRequest;

// ── IPC Response ────────────────────────────────────────────────────────

/// Daemon → Frontend response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Success with a JSON value payload, and for an intent that ran, how it
    /// can be taken back (plan D3.3: the undo toast and the CLI/TUI render
    /// it).
    Ok {
        data: serde_json::Value,
        reversibility: Option<rekindle_types::intent::Reversibility>,
    },
    /// The request is held by the host's intent gate (plan D3.3, ADR 0019):
    /// render this review and commit it with the proof it asks for.
    IntentRequired(rekindle_types::intent::IntentReview),
    /// Error with code, message, and optional remediation advice.
    Error {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
}

impl IpcResponse {
    /// Create a success response from a serializable value.
    pub fn ok<T: Serialize>(value: &T) -> Self {
        // [RC-2] serde_json::to_value can fail on recursive structures,
        // but our types are flat. If it fails, return an internal error.
        match serde_json::to_value(value) {
            std::result::Result::Ok(data) => Self::Ok {
                data,
                reversibility: None,
            },
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
/// - Media fan-out: `BusPayload::Media(MediaFrame)` (daemon → client only)
///
/// # Wire compatibility
///
/// Postcard encodes this enum by variant **index** (externally tagged), so
/// variants are only ever **appended** — never inserted or reordered — to keep
/// existing indices stable. `Media` is the last variant for that reason.
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
    /// Host → frontend stream frame (plan D3.2): a journaled event with its
    /// cursor, `Resumed` after a replay, or `InvalidSession`. Each
    /// subscribed connection has its own ordered stream from the journal.
    Event(rekindle_types::subscription_events::StreamFrame),
    /// Daemon → Frontend compressed video frame.
    ///
    /// A sibling of `Event`, deliberately **not** an event: media bypasses the
    /// EventRouter's subscription routing and each connection's ordered event
    /// queue, and rides its own bounded, drop-oldest queue at each hop,
    /// because a late video frame is worthless (unlike an event, which must
    /// arrive in order). Postcard-
    /// native — `MediaFrame` never crosses the JSON boundary `Response` does.
    ///
    /// Daemon → client only: a client never sends `Media`, and a client-origin
    /// `Media` is rejected by the server router (see `ipc/server/routing.rs`).
    Media(rekindle_types::video::MediaFrame),
}
