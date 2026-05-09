//! IPC message envelope.
//!
//! The `Message<T>` type wraps any payload with routing metadata, security
//! classification, and agent identity. The bus server stamps `verified_sender_name`
//! after Noise handshake verification — clients can never forge this field.
//!
//! [RC-16] `Debug` impl redacts payload for messages that may contain secrets.


use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current wire format version. Increment on field addition.
///
/// Postcard positional encoding: new fields appended to the struct are
/// invisible to old receivers (they stop reading at their known field count).
/// Receivers check `wire_version` before interpreting trailing fields.
pub const WIRE_VERSION: u8 = 1;

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
        // 2^64 ms = 584 million years. The daemon will not run that long;
        // saturate to u64::MAX in the impossible case rather than #[allow].
        let monotonic_ms = u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        let wall_ms = rekindle_utils::timestamp_ms();
        Self {
            monotonic_ms,
            wall_ms,
        }
    }
}

/// The IPC bus message envelope wrapping any payload type.
#[derive(Clone, Serialize, Deserialize)]
pub struct Message<T> {
    /// Wire format version. Always serialized first.
    pub wire_version: u8,
    /// Unique message identifier (UUID v7 for time-ordering).
    pub msg_id: Uuid,
    /// Correlation ID for request-response patterns.
    pub correlation_id: Option<Uuid>,
    /// Sender's agent identity.
    pub sender: Uuid,
    /// Dual-clock timestamp.
    pub timestamp: Timestamp,
    /// The application payload.
    pub payload: T,
    /// Access control classification.
    pub security_level: SecurityLevel,
    /// Server-stamped verified sender name. NEVER set by clients.
    pub verified_sender_name: Option<String>,
    /// Agent type classification.
    pub agent_type: Option<AgentType>,
    /// Community scope for event routing (governance key, or None for global).
    pub community_scope: Option<String>,
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
            timestamp: Timestamp::now(epoch),
            payload,
            security_level: level,
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
            .field("verified_sender_name", &self.verified_sender_name)
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
}
