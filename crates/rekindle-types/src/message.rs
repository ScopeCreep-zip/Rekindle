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
    /// which also orders our edits (rule E1).
    Edit {
        target: String,
        body: String,
        edited_at: u64,
    },
    /// A delete of our post `target` on our outbox (rule E3, a tombstone).
    Delete { target: String, deleted_at: u64 },
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

    /// The poll a poll op (create, vote, close) is of: a vote or close
    /// names its id; a create names only its nonce, and the id is derived
    /// from it and the creator's pseudonym (`rekindle_channel::poll_id_of`).
    #[must_use]
    pub fn poll_ref(&self) -> Option<PollRef<'_>> {
        match self {
            Self::PollCreate { nonce, .. } => Some(PollRef::Created { nonce }),
            Self::PollVote { poll_id, .. } | Self::PollClose { poll_id, .. } => {
                Some(PollRef::Id(poll_id))
            }
            _ => None,
        }
    }
}

/// How a poll op names its poll ([`PostEntry::poll_ref`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollRef<'a> {
    /// A vote or close: the poll's id.
    Id(&'a [u8; 16]),
    /// A create: the creator's nonce, which with the creator's pseudonym
    /// derives the poll's id.
    Created { nonce: &'a [u8; 16] },
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

/// The `app_call` reply to a call its callee could not judge or apply in
/// time (its deadline, `CALL_TIMEOUT_MS` less a second) or at all (its own
/// fault): one byte, never a receipt (empty), a NAK (`N`, the refusal), a
/// `'D'` frame or an encoded envelope. A deadline is no refusal (review
/// RV5 H1, M2): the caller attempts the call again as on a timeout. Here
/// so a caller below the transport (the join's bootstrap,
/// `rekindle-governance-runtime`) reads the same byte the transport
/// answers (`rekindle_transport::operations::direct::RETRY_LATER`).
pub const CALL_RETRY_LATER: &[u8] = b"R";

/// How far in the future a signed 1:1 envelope's timestamp may be (clock
/// skew between the two peers).
pub const ENVELOPE_MAX_FUTURE_SKEW_MS: u64 = 60_000;

// ── Phase 9 — Per-class message taxonomy for SafetyProfile selection ──

/// The traffic classes of Rekindle's sends, one per row of
/// [`crate::config::SafetyConfig`], the one class-to-profile table
/// ([`crate::config::SafetyConfig::for_class`]; its defaults are
/// `rekindle_route::profile::profile_for_class`). Every class rides a Veilid
/// safety route at [`crate::config::ANONYMITY_HOP_FLOOR`]; a class tunes
/// only stability and sequencing, never anonymity.
///
/// Control traffic is ordered and media unordered (plan VU decision 2), as
/// Veilid intends: a routed operation carries the sender's sequencing and
/// every hop filters the next hop by it (veilid-core
/// `rpc_processor/rpc_route.rs:26-29`, `:87-90`), while a receiver's
/// General route had its hops chosen and proven on its one ordering,
/// Ordered (`route_allocate.rs:580-588`). An unordered send to it crosses
/// UDP hop pairs nobody proved (real-app DL1/DL2, 2026-11-25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageClass {
    /// 1:1 control traffic: DMs and their receipts, friend handshakes,
    /// presence keys, group invites, call signals (the transport's
    /// `send_direct`/`call_direct`). Reliable, PreferOrdered.
    Text,
    /// Voice and video frames inside a call, sent to the peer's Media route
    /// (itself LowLatency, PreferUnordered). LowLatency, PreferUnordered:
    /// the jitter buffer reorders and a late frame is useless.
    Voice,
    /// Signed RPCs, community envelopes and gossip (`AppCaller::call`,
    /// `call_community_envelope`, `send_unframed_to_route`). Reliable,
    /// EnsureOrdered.
    Rpc,
    /// DHT record operations: the record pool's safety selection
    /// (`VeilidPorts::start`). Reliable, PreferOrdered.
    Dht,
}

impl MessageClass {
    /// Human-readable label for tracing.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Rpc => "rpc",
            Self::Dht => "dht",
        }
    }
}

/// A receipted send that went unanswered and is being tried again (plan
/// §3.0 lane 1): how many attempts have gone out and when the next one is
/// due (ms since the epoch). The UI shows the message as not delivered yet,
/// still trying; the first answer clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryRetry {
    pub attempts: u32,
    pub next_attempt_at: i64,
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
