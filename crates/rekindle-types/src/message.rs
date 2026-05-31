//! Architecture §13.4 / §15 — channel-message snapshots carried inside
//! `BootstrapResponse.recent_messages` and `SyncResponse.messages`. Wire
//! shapes preserved exactly from the pre-migration JSON form so the
//! envelope swap is wire-compatible; the Cap'n Proto migration replaces
//! these with typed schemas.

use serde::{Deserialize, Serialize};

/// One MEK-encrypted message inside a `BootstrapChannelMessages` block.
/// Architecture §13.4 line 2068 — `ciphertext` is freshly re-encrypted
/// under the joiner's current MEK (the same key delivered alongside in
/// `BootstrapResponse.channel_meks`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapMessage {
    pub message_id: String,
    /// Hex-encoded sender pseudonym.
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: i64,
}

/// Architecture §13.4 — bootstrap snapshot grouped by channel so the
/// joiner doesn't pay the per-message overhead of repeating
/// `channel_id` for every entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapChannelMessages {
    pub channel_id: String,
    pub messages: Vec<BootstrapMessage>,
}

/// Architecture §15 — sync-response message entry. Different shape from
/// `BootstrapMessage` because it's pulled from SQLite, where the
/// historical message rows carry the columns this struct mirrors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SyncedMessage {
    /// Hex-encoded sender community pseudonym.
    pub sender_key: String,
    /// Stored message body (architecture §15 line 2210 — already
    /// MEK-encrypted at write time).
    pub body: String,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mek_generation: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamport_ts: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_message_roundtrip() {
        let m = BootstrapMessage {
            message_id: "msg_01".into(),
            sender_pseudonym: "abcd".into(),
            ciphertext: vec![1, 2, 3, 4],
            mek_generation: 1,
            timestamp: 100,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: BootstrapMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn bootstrap_channel_messages_roundtrip() {
        let g = BootstrapChannelMessages {
            channel_id: "ch_01".into(),
            messages: vec![BootstrapMessage {
                message_id: "msg_01".into(),
                sender_pseudonym: "abcd".into(),
                ciphertext: vec![1, 2, 3],
                mek_generation: 1,
                timestamp: 100,
            }],
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: BootstrapChannelMessages = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn synced_message_roundtrip() {
        let m = SyncedMessage {
            sender_key: "abcd".into(),
            body: "hello".into(),
            timestamp: 100,
            mek_generation: Some(1),
            lamport_ts: Some(42),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: SyncedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn synced_message_omits_optional_when_absent() {
        let m = SyncedMessage {
            sender_key: "abcd".into(),
            body: "hello".into(),
            timestamp: 100,
            mek_generation: None,
            lamport_ts: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("mek_generation"));
        assert!(!json.contains("lamport_ts"));
    }
}

// ── Phase 9 — Per-class message taxonomy for SafetyProfile selection ──

/// Per-class message taxonomy. Carried alongside the payload through
/// `Sender::send_dm(class, …)` so the transport layer can pick the
/// appropriate `SafetyProfile` without reaching back into the call site.
///
/// Different message categories want different privacy/latency trade-offs
/// at the Veilid routing layer:
/// - **Text** DMs prioritize anonymity over latency (2-hop relay).
/// - **Voice** frames must be low-latency (0 hops, direct route).
/// - **Rpc** invites need reliability but not strict order.
/// - **DhtRead/DhtWrite** are infrastructure operations.
///
/// [`crate::config::SafetyProfile`] holds the actual parameters;
/// `rekindle-route::profile::profile_for_class` is the lookup function.
///
/// Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageClass {
    /// 1:1 text DMs and friend-graph control messages. 2-hop relay
    /// for sender anonymity; ordered + reliable.
    Text,
    /// Voice frames inside an active audio call. 0-hop direct route
    /// for minimum latency; unordered (jitter buffer reorders).
    Voice,
    /// RPC invites and other one-shot reply-required messages.
    /// 1-hop relay; reliable; no ordering requirement.
    Rpc,
    /// DHT subkey reads (profile, prekey, route). 1-hop relay.
    DhtRead,
    /// DHT subkey writes (publish profile, rotate prekey).
    /// 2-hop relay for write anonymity; ordered + reliable.
    DhtWrite,
}

impl MessageClass {
    /// Human-readable label for tracing.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Rpc => "rpc",
            Self::DhtRead => "dht_read",
            Self::DhtWrite => "dht_write",
        }
    }
}
