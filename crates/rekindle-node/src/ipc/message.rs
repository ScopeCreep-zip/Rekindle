//! IPC message envelope.
//!
//! The `Message<T>` type wraps any payload with routing metadata, security
//! classification, and agent identity. The bus server stamps `verified_sender_name`
//! after Noise handshake verification — clients can never forge this field.
//!
//! [RC-16] `Debug` impl redacts payload for messages that may contain secrets.


use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current wire format version. Increment on breaking layout changes.
///
/// Postcard positional encoding: fields are serialized in struct declaration
/// order. Changing field order is a breaking change requiring a version bump.
///
/// Version history:
/// - v1: original field order (security_level after payload)
/// - v2: security_level moved before timestamp+payload for partial deser
/// - v3: lane byte protocol — 1-byte prefix multiplexes control + bulk
pub const WIRE_VERSION: u8 = 3;

/// Security level classification for IPC messages.
///
/// Ordered by privilege: higher discriminant = more privileged.
/// Sender's clearance must be ≥ message's level. [RC-6]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SecurityLevel {
    /// Ephemeral CLI clients, unregistered keys.
    Open = 0,
    /// Human frontends after passphrase unlock.
    Authenticated = 1,
    /// Registered agents within capability scope.
    Agent = 2,
    /// Trusted system daemons, relay bridges.
    Internal = 3,
    /// Multi-factor ceremony, time-limited TTL.
    Admin = 4,
}

/// Agent type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    /// Human user operating CLI, TUI, or Tauri.
    Human,
    /// Autonomous AI/LLM agent.
    AiLlm,
    /// Scheduled automation bot.
    Bot,
    /// Content filter (automod, spam, NSFW).
    Filter,
    /// Analysis service (metrics, sentiment).
    Analyzer,
    /// Protocol bridge (Matrix, IRC, Slack).
    Bridge,
    /// Internal system service (the daemon itself).
    System,
}

/// Dual-clock timestamp: monotonic (for ordering) + wall (for display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timestamp {
    /// Milliseconds since the daemon's monotonic epoch.
    pub monotonic_ms: u64,
    /// Wall clock milliseconds since Unix epoch (best-effort, not authoritative).
    pub wall_ms: u64,
}

impl Timestamp {
    /// Create a timestamp from the daemon's epoch.
    #[must_use]
    pub fn now(epoch: Instant) -> Self {
        // Monotonic and wall clock milliseconds. u128→u64 truncation is safe:
        // 2^64 ms = 584 million years. The daemon will not run that long.
        #[allow(clippy::cast_possible_truncation)]
        let monotonic_ms = epoch.elapsed().as_millis() as u64;
        #[allow(clippy::cast_possible_truncation)]
        let wall_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            monotonic_ms,
            wall_ms,
        }
    }
}

/// The IPC bus message envelope wrapping any payload type.
///
/// Field order is significant — postcard serializes positionally.
/// Fields 0-4 (wire_version through security_level) form the "routing
/// header" that the server can parse via `postcard::take_from_bytes`
/// without deserializing the payload. Do not reorder without bumping
/// `WIRE_VERSION`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Message<T> {
    /// Wire format version. Always serialized first. Position 0.
    pub wire_version: u8,
    /// Unique message identifier (UUID v7 for time-ordering). Position 1.
    pub msg_id: Uuid,
    /// Correlation ID for request-response patterns. Position 2.
    pub correlation_id: Option<Uuid>,
    /// Sender's agent identity. Position 3.
    pub sender: Uuid,
    /// Access control classification. Position 4.
    /// Placed before timestamp+payload so routing headers can be parsed
    /// without deserializing the payload (postcard positional encoding).
    pub security_level: SecurityLevel,
    /// Dual-clock timestamp. Position 5.
    pub timestamp: Timestamp,
    /// The application payload. Position 6.
    pub payload: T,
    /// Server-stamped verified sender name. NEVER set by clients. Position 7.
    /// `Arc<str>` for zero-cost clone during fan-out — the server stamps
    /// the same name on every frame from a connection.
    pub verified_sender_name: Option<Arc<str>>,
    /// Agent type classification. Position 8.
    pub agent_type: Option<AgentType>,
    /// Community scope for event routing (governance key, or None for global). Position 9.
    pub community_scope: Option<String>,
}

/// Routing header — prefix of `Message<T>` for partial deserialization.
///
/// Contains only fields 0-4 of `Message`. `postcard::take_from_bytes`
/// can deserialize this from a `Message` byte stream without parsing
/// the timestamp, payload, or trailing fields. Used by the server to
/// extract routing metadata for daemon-bound request forwarding without
/// full frame decode+re-encode.
/// Fields 0-5 of `Message`. After `postcard::take_from_bytes::<RoutingHeader>`,
/// the remaining bytes start with the `BusPayload` discriminant — enabling
/// discriminant-only routing without full payload deserialization.
#[derive(Debug, Clone, Deserialize)]
pub struct RoutingHeader {
    pub wire_version: u8,
    pub msg_id: Uuid,
    pub correlation_id: Option<Uuid>,
    pub sender: Uuid,
    pub security_level: SecurityLevel,
    /// Included so `take_from_bytes` consumes through the timestamp,
    /// leaving the remaining bytes starting at the `BusPayload` discriminant.
    pub timestamp: Timestamp,
}

/// Frame forwarded from server to daemon subscriber.
///
/// Separates routing metadata (cheap to extract via `take_from_bytes`)
/// from the raw payload (forwarded without re-serialization). The daemon
/// subscriber only calls `postcard::from_bytes::<Message<BusPayload>>`
/// on `raw` when it needs to dispatch on the payload variant.
///
/// This eliminates the full decode+re-encode that `route_frame` previously
/// did for every daemon-bound request frame.
#[derive(Debug, Clone)]
pub struct RoutedFrame {
    /// Routing metadata parsed from the first 5 fields of the message.
    pub header: RoutingHeader,
    /// Verified sender name stamped by the server from connection state.
    /// Not in the raw bytes — passed out-of-band.
    pub verified_sender_name: Option<Arc<str>>,
    /// The raw postcard bytes of the full `Message<BusPayload>`.
    /// Forwarded without re-encoding. The daemon deserializes only when
    /// it needs to inspect the payload.
    pub raw: bytes::Bytes,
}

/// Zero-vtable shared frame for event fan-out.
///
/// `Bytes::clone()` is 14.5ns (vtable dispatch + atomic refcount).
/// `SharedFrame::clone()` is ~5ns (direct `Arc::clone` — single `fetch_add`,
/// no vtable, compiler can inline).
///
/// At 50K subscribers, this saves 475μs per event (50K × 9.5ns).
///
/// Use `SharedFrame` for the event delivery channel (hot fan-out path).
/// Use `Bytes` for the read/write I/O path (NoiseTransport, client channels).
#[derive(Clone)]
pub struct SharedFrame(Arc<[u8]>);

impl SharedFrame {
    /// Create from `Bytes`. Copies the bytes into a new `Arc<[u8]>` allocation.
    /// This copy happens once per event (in `serialize_event`). All subsequent
    /// clones during fan-out are zero-cost (`Arc::clone` = atomic increment).
    pub fn from_bytes(b: &[u8]) -> Self {
        Self(Arc::from(b))
    }
}

impl Deref for SharedFrame {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SharedFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SharedFrame").field(&self.0.len()).finish()
    }
}

/// Context for constructing outbound messages.
#[derive(Debug, Clone)]
pub struct MessageContext {
    pub sender: Uuid,
    pub agent_type: Option<AgentType>,
}

impl MessageContext {
    /// Create a context with just a sender ID.
    #[must_use]
    pub fn new(sender: Uuid) -> Self {
        Self {
            sender,
            agent_type: None,
        }
    }

    /// Create a context with sender ID and agent type.
    #[must_use]
    pub fn with_type(sender: Uuid, agent_type: AgentType) -> Self {
        Self {
            sender,
            agent_type: Some(agent_type),
        }
    }
}

impl<T: Serialize> Message<T> {
    /// Create a new message with a fresh UUID v7 and current timestamp.
    #[must_use]
    pub fn new(ctx: &MessageContext, payload: T, level: SecurityLevel, epoch: Instant) -> Self {
        Self {
            wire_version: WIRE_VERSION,
            msg_id: Uuid::now_v7(),
            correlation_id: None,
            sender: ctx.sender,
            security_level: level,
            timestamp: Timestamp::now(epoch),
            payload,
            verified_sender_name: None, // Server fills this
            agent_type: ctx.agent_type,
            community_scope: None,
        }
    }

    /// Set a correlation ID for request-response linking.
    #[must_use]
    pub fn with_correlation(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set community scope for targeted routing.
    #[must_use]
    pub fn with_community(mut self, gov_key: String) -> Self {
        self.community_scope = Some(gov_key);
        self
    }
}

/// [RC-16] Custom Debug that redacts payload for secret-bearing messages.
/// Delegates to T's Debug — T must implement Debug with secret redaction.
impl<T: fmt::Debug> fmt::Debug for Message<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("wire_version", &self.wire_version)
            .field("msg_id", &self.msg_id)
            .field("correlation_id", &self.correlation_id)
            .field("sender", &self.sender)
            .field("security_level", &self.security_level)
            .field("verified_sender_name", &self.verified_sender_name.as_deref())
            .field("agent_type", &self.agent_type)
            .field("community_scope", &self.community_scope)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> MessageContext {
        MessageContext::new(Uuid::nil())
    }

    #[test]
    fn message_new_sets_wire_version() {
        let msg = Message::new(&test_ctx(), "hello", SecurityLevel::Open, Instant::now());
        assert_eq!(msg.wire_version, WIRE_VERSION);
    }

    #[test]
    fn message_new_defaults_are_safe() {
        let msg = Message::new(&test_ctx(), 42u32, SecurityLevel::Open, Instant::now());
        // [RC-2] verified_sender_name must be None on construction.
        assert!(msg.verified_sender_name.is_none());
        // correlation_id must be None on new requests.
        assert!(msg.correlation_id.is_none());
    }

    #[test]
    fn message_roundtrip_preserves_wire_version() {
        let msg = Message::new(&test_ctx(), "test", SecurityLevel::Authenticated, Instant::now());
        let bytes = super::super::framing::encode_frame(&msg).unwrap();
        let decoded: Message<String> = super::super::framing::decode_frame(&bytes).unwrap();
        assert_eq!(decoded.wire_version, WIRE_VERSION);
        assert_eq!(decoded.payload, "test");
    }

    #[test]
    fn with_correlation_sets_id() {
        let msg = Message::new(&test_ctx(), 0u8, SecurityLevel::Open, Instant::now());
        let corr = Uuid::now_v7();
        let msg = msg.with_correlation(corr);
        assert_eq!(msg.correlation_id, Some(corr));
    }

    #[test]
    fn security_level_ordering() {
        assert!(SecurityLevel::Open < SecurityLevel::Authenticated);
        assert!(SecurityLevel::Authenticated < SecurityLevel::Agent);
        assert!(SecurityLevel::Agent < SecurityLevel::Internal);
        assert!(SecurityLevel::Internal < SecurityLevel::Admin);
    }

    #[test]
    fn routing_header_matches_message_field_order() {
        // RoutingHeader must deserialize from the first N fields of
        // Message<BusPayload> via postcard::take_from_bytes. If the
        // field order diverges, this test fails.
        use crate::ipc::protocol::{BusPayload, IpcRequest};

        let corr = Uuid::now_v7();
        let sender = Uuid::now_v7();
        let ctx = MessageContext::new(sender);
        let msg = Message::new(
            &ctx,
            BusPayload::Request(IpcRequest::Status),
            SecurityLevel::Authenticated,
            Instant::now(),
        ).with_correlation(corr);

        let bytes = crate::ipc::framing::encode_frame(&msg).unwrap();
        let (header, _remaining) =
            postcard::take_from_bytes::<RoutingHeader>(&bytes).unwrap();

        assert_eq!(header.wire_version, WIRE_VERSION);
        assert_eq!(header.msg_id, msg.msg_id);
        assert_eq!(header.correlation_id, Some(corr));
        assert_eq!(header.sender, sender);
        assert_eq!(header.security_level, SecurityLevel::Authenticated);
        // Timestamp monotonic_ms should be within a reasonable range
        // (not zero, not garbage from misaligned fields).
        assert!(header.timestamp.monotonic_ms < 10_000);
    }
}
