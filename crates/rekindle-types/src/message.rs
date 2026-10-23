//! Architecture §15 — channel-message snapshots carried inside
//! `SyncResponse.messages`. Wire
//! shapes preserved exactly from the pre-migration JSON form so the
//! envelope swap is wire-compatible; the Cap'n Proto migration replaces
//! these with typed schemas.

use serde::{Deserialize, Serialize};

/// Architecture §15 — sync-response message entry. It is pulled from SQLite, where the
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

/// Our community channel post and the record slot its write targets (plan
/// E2.5, C7.14): the payload of its outbox `channel_post` row, which
/// settles when that slot's write does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPost {
    pub community: String,
    pub channel: String,
    /// The post's client id, its row's.
    pub message: String,
    pub record_key: String,
    pub subkey: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

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

// ── Signed 1:1 envelope freshness ──

/// How old a signed 1:1 envelope may be when it arrives, on both tracks.
/// Senders sign at send time (a retry re-signs), so this only has to cover
/// transit and clock skew; anything older is a replay.
pub const ENVELOPE_FRESHNESS_WINDOW_MS: u64 = 300_000;

/// The bounds of a `DmAck`'s coalescing delay, ms (plan E2.5, r10 Part A:
/// "coalesced per peer and sent after a uniform random 2–10 s delay").
pub const DM_ACK_DELAY_MIN_MS: u64 = 2_000;
pub const DM_ACK_DELAY_MAX_MS: u64 = 10_000;

/// Veilid's RPC round-trip timeout, ms (`network.rpc.timeout_ms`, default
/// 5 000, `veilid-core/src/veilid_config.rs:852`): one call's longest wait,
/// as an ack's send (an `app_message` after an inline route refresh) may
/// take.
pub const CALL_TIMEOUT_MS: u64 = 5_000;

/// How far in the future a signed 1:1 envelope's timestamp may be (clock
/// skew between the two peers).
pub const ENVELOPE_MAX_FUTURE_SKEW_MS: u64 = 60_000;

// ── Phase 9 — Per-class message taxonomy for SafetyProfile selection ──

/// Per-class message taxonomy: the class selects the `SafetyProfile` a
/// message is sent under, so the transport layer can pick it without
/// reaching back into the call site.
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

/// A sent message's delivery state: D22's one vocabulary, stored in
/// `messages.send_state` (plan D1, C7.14). A channel post moves `Queued`
/// (written locally, not yet at consensus) → `Sent` (its record slot
/// settled with it) or `Failed`; it never reaches `Delivered`/`Read`,
/// because a community has no per-reader receipts. A received message is
/// `Delivered`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendState {
    Queued,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl SendState {
    /// The stored and wire form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
        }
    }

    /// The state a stored form names.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        [
            Self::Queued,
            Self::Sent,
            Self::Delivered,
            Self::Read,
            Self::Failed,
        ]
        .into_iter()
        .find(|s| s.as_str() == stored)
    }
}

/// The conversation a `messages` row belongs to (`messages.conversation_type`,
/// plan E2.6): a 1:1 DM, a group DM, or a community channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationType {
    Dm,
    GroupDm,
    Channel,
}

impl ConversationType {
    /// The stored form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::GroupDm => "group_dm",
            Self::Channel => "channel",
        }
    }
}

impl std::str::FromStr for ConversationType {
    type Err = String;

    fn from_str(stored: &str) -> Result<Self, Self::Err> {
        [Self::Dm, Self::GroupDm, Self::Channel]
            .into_iter()
            .find(|t| t.as_str() == stored)
            .ok_or_else(|| format!("unknown conversation type {stored:?}"))
    }
}

#[cfg(test)]
mod conversation_type_tests {
    use super::ConversationType;

    #[test]
    fn every_type_round_trips_and_others_are_refused() {
        for t in [
            ConversationType::Dm,
            ConversationType::GroupDm,
            ConversationType::Channel,
        ] {
            assert_eq!(t.as_str().parse::<ConversationType>(), Ok(t));
        }
        assert!("group".parse::<ConversationType>().is_err());
    }
}
