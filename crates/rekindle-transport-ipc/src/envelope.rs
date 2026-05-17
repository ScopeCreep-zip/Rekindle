//! IPC message envelope.
//!
//! `Message<T>` wraps any payload with routing metadata, security
//! classification, and agent identity. The bus server stamps
//! `verified_sender_name` after Noise handshake — clients cannot forge it.
//!
//! Field order is significant — postcard serializes positionally.
//! Fields 0-5 form the "routing header" parseable via
//! `postcard::take_from_bytes` without deserializing the payload.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current wire format version. Increment on breaking layout changes.
pub const WIRE_VERSION: u8 = 3;

/// Security level classification for IPC messages.
///
/// Ordered by privilege: higher discriminant = more privileged.
/// Sender's clearance must be >= message's level.
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
    Human,
    AiLlm,
    Bot,
    Filter,
    Analyzer,
    Bridge,
    System,
}

/// Dual-clock timestamp: monotonic (ordering) + wall (display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timestamp {
    /// Milliseconds since the daemon's monotonic epoch.
    pub monotonic_ms: u64,
    /// Wall clock milliseconds since Unix epoch (best-effort).
    pub wall_ms: u64,
}

impl Timestamp {
    /// Create a timestamp from the daemon's epoch.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn now(epoch: Instant) -> Self {
        let monotonic_ms = epoch.elapsed().as_millis() as u64;
        let wall_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self { monotonic_ms, wall_ms }
    }
}

/// The IPC bus message envelope wrapping any payload type `T`.
///
/// Field order is significant — postcard serializes positionally.
/// Fields 0-5 (wire_version through timestamp) form the routing header
/// that the server can parse via `postcard::take_from_bytes` without
/// deserializing the payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct Message<T> {
    /// Wire format version. Position 0.
    pub wire_version: u8,
    /// Unique message identifier (UUID v7 for time-ordering). Position 1.
    pub msg_id: Uuid,
    /// Correlation ID for request-response patterns. Position 2.
    pub correlation_id: Option<Uuid>,
    /// Sender's agent identity. Position 3.
    pub sender: Uuid,
    /// Access control classification. Position 4.
    pub security_level: SecurityLevel,
    /// Dual-clock timestamp. Position 5.
    pub timestamp: Timestamp,
    /// The application payload. Position 6.
    pub payload: T,
    /// Server-stamped verified sender name. NEVER set by clients. Position 7.
    pub verified_sender_name: Option<Arc<str>>,
    /// Agent type classification. Position 8.
    pub agent_type: Option<AgentType>,
    /// Community scope for event routing. Position 9.
    pub community_scope: Option<String>,
}

/// Routing header — prefix of `Message<T>` for partial deserialization.
///
/// `postcard::take_from_bytes::<RoutingHeader>` deserializes fields 0-5
/// from a `Message` byte stream without parsing the payload.
#[derive(Debug, Clone, Deserialize)]
pub struct RoutingHeader {
    pub wire_version: u8,
    pub msg_id: Uuid,
    pub correlation_id: Option<Uuid>,
    pub sender: Uuid,
    pub security_level: SecurityLevel,
    pub timestamp: Timestamp,
}

/// Frame forwarded from server to application subscriber.
///
/// Separates routing metadata (cheap partial deser) from the raw
/// payload (forwarded without re-serialization).
#[derive(Debug, Clone)]
pub struct RoutedFrame {
    /// Routing metadata parsed from the first 6 fields of the message.
    pub header: RoutingHeader,
    /// Verified sender name stamped by the server. Not in the raw bytes.
    pub verified_sender_name: Option<Arc<str>>,
    /// Raw postcard bytes of the full `Message<T>`.
    pub raw: bytes::Bytes,
}

/// Zero-vtable shared frame for event fan-out.
///
/// `Arc::clone` ~5ns (single fetch_add, no vtable).
/// `Bytes::clone` ~14.5ns (vtable dispatch + atomic refcount).
/// At 50K subscribers, saves 475us per event.
#[derive(Clone)]
pub struct SharedFrame(Arc<[u8]>);

impl SharedFrame {
    /// Create from a byte slice. One copy; all subsequent clones are zero-cost.
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
    #[must_use]
    pub fn new(sender: Uuid) -> Self {
        Self { sender, agent_type: None }
    }

    #[must_use]
    pub fn with_type(sender: Uuid, agent_type: AgentType) -> Self {
        Self { sender, agent_type: Some(agent_type) }
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
            verified_sender_name: None,
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

impl<T: fmt::Debug> fmt::Debug for Message<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("wire_version", &self.wire_version)
            .field("msg_id", &self.msg_id)
            .field("correlation_id", &self.correlation_id)
            .field("sender", &self.sender)
            .field("security_level", &self.security_level)
            .field("verified_sender_name", &self.verified_sender_name.as_deref())
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_defaults_are_safe() {
        let ctx = MessageContext::new(Uuid::nil());
        let msg = Message::new(&ctx, "hello", SecurityLevel::Open, Instant::now());
        assert!(msg.verified_sender_name.is_none());
        assert!(msg.correlation_id.is_none());
        assert_eq!(msg.wire_version, WIRE_VERSION);
    }

    #[test]
    fn security_level_ordering() {
        assert!(SecurityLevel::Open < SecurityLevel::Authenticated);
        assert!(SecurityLevel::Authenticated < SecurityLevel::Agent);
        assert!(SecurityLevel::Agent < SecurityLevel::Internal);
        assert!(SecurityLevel::Internal < SecurityLevel::Admin);
    }

    #[test]
    fn message_roundtrip_preserves_wire_version() {
        let ctx = MessageContext::new(Uuid::nil());
        let msg = Message::new(&ctx, "test", SecurityLevel::Authenticated, Instant::now());
        let bytes = crate::frame::codec::encode_frame(&msg).unwrap();
        let decoded: Message<String> = crate::frame::codec::decode_frame(&bytes).unwrap();
        assert_eq!(decoded.wire_version, WIRE_VERSION);
        assert_eq!(decoded.payload, "test");
    }

    #[test]
    fn routing_header_matches_message_field_order() {
        let ctx = MessageContext::new(Uuid::now_v7());
        let msg = Message::new(&ctx, 42u32, SecurityLevel::Agent, Instant::now());
        let bytes = crate::frame::codec::encode_frame(&msg).unwrap();
        let (header, _remaining) =
            postcard::take_from_bytes::<RoutingHeader>(&bytes).unwrap();
        assert_eq!(header.wire_version, WIRE_VERSION);
        assert_eq!(header.sender, ctx.sender);
        assert_eq!(header.security_level, SecurityLevel::Agent);
    }
}
