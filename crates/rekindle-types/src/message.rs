//! Our community channel posts: the payloads of their outbox
//! `channel_post` rows (plan E3.3).

use serde::{Deserialize, Serialize};

/// Our community channel post (plan E3.3): the payload of its outbox
/// `channel_post` row. A message or forward is an entry of our member
/// record's channel outbox (`M`/4), sealed when the stream's sender drains
/// it; an attachment message too, its offer carried on the row (E3.3
/// slice 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPost {
    pub community: String,
    pub channel: String,
    /// The post's client id, its row's (for an edit or delete, the
    /// change's own row id; its target is in [`PostEntry`]).
    pub message: String,
    pub entry: PostEntry,
}

/// What a `channel_post` row writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PostEntry {
    /// A message entry on our outbox, its body read from our row.
    Message,
    /// A forward entry on our outbox: our row's body is the snapshot of
    /// `original_message` by `original_author` in `original_channel`.
    Forward {
        original_channel: String,
        original_message: String,
        original_author: String,
    },
    /// An attachment message on our outbox (plan E3.3 slice 7): a message
    /// entry carrying `offer` (its FEK wrapped at upload) and `flags`
    /// (a voice message's), its body read from our row.
    Attachment {
        offer: crate::attachment::AttachmentOffer,
        flags: u32,
    },
    /// We hold every chunk of the attachment `attachment_id` (its offer's
    /// `chunk_count` chunks), downloaded: an `AttachmentCached` entry on our
    /// outbox (plan E3.3 slice 7), which a downloader reads to find its
    /// sources. Its row has no echo. The uploader writes none: the offer on
    /// its own message says it holds the file (`docs/architecture/files.md`).
    Cached {
        attachment_id: [u8; 16],
        chunk_count: u32,
    },
    /// An edit of our post `target` on our outbox (plan E3.3): its new
    /// body, sealed when the stream drains it at the entry's stream seq,
    /// and our seq `seq` that orders our edits (rule E1).
    Edit {
        target: String,
        body: String,
        edited_at: u64,
        seq: u64,
    },
    /// A delete of our post `target` on our outbox (rule E3, a tombstone).
    Delete {
        target: String,
        deleted_at: u64,
        seq: u64,
    },
    /// Our reaction `emoji` added to or removed from `target` (rule R1: a
    /// reaction op is (target, emoji, add|remove); readers keep our latest
    /// by its stream seq). No body.
    Reaction {
        target: String,
        emoji: String,
        added: bool,
    },
    /// Our poll attached to `target`, our message; its id is derived from
    /// our pseudonym and `nonce` (plan E3.3, rule E1).
    PollCreate {
        nonce: [u8; 16],
        target: String,
        question: String,
        answers: Vec<String>,
        multi_select: bool,
        expires_at: Option<u64>,
    },
    /// Our vote on the poll `poll_id` (our latest by stream seq counts).
    PollVote {
        poll_id: [u8; 16],
        selected_answers: Vec<u8>,
    },
    /// Our close of our poll `poll_id`, naming the votes it counts: each
    /// voter's pseudonym hex and the stream seq of the vote counted, as our
    /// reader held them when we closed (plan E3.3, rule E1).
    /// One part of the close (`part` of `parts`, sharing `close_id`): a
    /// close over more voters than one entry holds is several entries.
    PollClose {
        poll_id: [u8; 16],
        close_id: [u8; 16],
        part: u16,
        parts: u16,
        counted: Vec<(String, u64)>,
    },
    /// Our hand raised or lowered on a voice stage (our latest by stream
    /// seq stands).
    HandRaise { raised: bool },
    /// Our reply in the thread this post's `channel` names (plan E3.3: a
    /// thread is a stream whose tag is derived from its id), sealed when
    /// the stream drains it under the text key of `parent_channel`.
    ThreadReply {
        parent_channel: String,
        body: String,
        sent_at_ms: u64,
        /// The thread's standing manual archive (its op hash hex) when we
        /// replied: our reply revives it.
        revives: Option<String>,
    },
}

impl PostEntry {
    /// Whether the entry is a post with a `messages` row of its own (a
    /// message, forward or attachment message), rather than an op on
    /// other rows whose row id names no message.
    #[must_use]
    pub fn has_row(&self) -> bool {
        matches!(
            self,
            Self::Message | Self::Forward { .. } | Self::Attachment { .. }
        )
    }

    /// The poll a poll op (create, vote, close) is of.
    #[must_use]
    pub fn poll_id(&self) -> Option<&[u8; 16]> {
        match self {
            Self::PollCreate { poll_id, .. }
            | Self::PollVote { poll_id, .. }
            | Self::PollClose { poll_id, .. } => Some(poll_id),
            _ => None,
        }
    }
}

impl ChannelPost {
    /// The row's stream scope: one `channel_post` stream per channel
    /// (`<community>:<channel>`, in queue order).
    #[must_use]
    pub fn scope(&self) -> String {
        format!("{}:{}", self.community, self.channel)
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
/// 5 000, veilid-core 0.5.8 `veilid-core/src/veilid_config.rs:944`): one call's longest wait,
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
    /// A community post composed before we held its channel's key (plan
    /// E3.2 S7l, F9-1): stored with its outbox row, written once the key
    /// is installed. Below `Queued`: nothing has gone to the network.
    #[serde(rename = "awaiting_key")]
    AwaitingKey,
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
            Self::AwaitingKey => "awaiting_key",
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
            Self::AwaitingKey,
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
