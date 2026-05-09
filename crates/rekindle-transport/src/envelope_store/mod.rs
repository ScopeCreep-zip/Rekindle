//! W16.1 — `EnvelopeStore` persistence trait for the reliability primitive.
//!
//! Defines the storage contract used by `envelope_queue` (W16.2) and the
//! call state persistence (W16.8). Each frontend (Tauri shell, rekindle-cli,
//! rekindle-node) provides a concrete impl: SQLite for Tauri, atomic JSON
//! file for CLI/node, in-memory for tests.
//!
//! Trait is `dyn`-safe via `#[async_trait]` (native `async fn` in trait
//! is stable as of Rust 1.75 but not yet dyn-safe — the macro stays
//! standard for `Box<dyn Trait>` / `Arc<dyn Trait>` cases).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::frame::TypeId;

pub mod json;
pub mod memory;

pub use json::JsonEnvelopeStore;
pub use memory::MemoryEnvelopeStore;

/// Discriminator for an outbound envelope. Drives per-kind retry config
/// and is the dedup key on the receiver side (combined with sender +
/// correlation_id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvelopeKind {
    // ── 1:1 Call signaling — post-decision phase (W16 + W16.5b) ──
    //
    // CallInvite + CallRinging dropped per W16.5b: the wire-level
    // invite-and-ringing handshake travels via Veilid `app_call`
    // (5-10 s budget; matches SIP 100-Trying / 180-Ringing) — see
    // `payload::rpc::{CallInvitePayload, CallRingingPayload}` and
    // `operations::calls::CallRuntime::interpret_one(SendCallInvite)`.
    // The `EnvelopeQueue` only handles the user-decision response
    // (Accept/Decline/End) which is unbounded by Veilid's RPC budget.
    /// Receiver → caller: accepted; carries acceptor X25519 pub.
    CallAccept,
    /// Receiver → caller: declined.
    CallDecline,
    /// Either side: hangup or cancel.
    CallEnd,
    /// Mid-call: peer toggled mic / camera / screen.
    CallMediaState,
    /// Mid-call: emoji reaction.
    CallReaction,
    /// Caller → invitee: group call invite (per-invitee fan-out).
    GroupCallOffer,
    /// Invitee → caller: group call accept.
    GroupCallAccept,
    /// Invitee → caller: group call decline.
    GroupCallDecline,

    // ── Friend-add (3-phase DHT-inbox handshake — W16.10) ──
    /// Initiator writes FriendRequestEntry to target's profile inbox.
    FriendRequestInbox,
    /// Target writes FriendAcceptEntry to initiator's profile inbox.
    FriendAcceptInbox,

    // ── DM body content (Signal-encrypted) ──
    /// Direct message body via Signal Double Ratchet.
    DmMessage,

    // ── Expect-reply primitive (W16.10b + future MEK / files / bootstrap) ──
    /// DM invite request — initiator awaits a reply with the new DM record key.
    DmInviteRequest,
    /// DM invite reply — receiver's accept/decline + DM record key on accept.
    DmInviteReply,
    /// Group DM invite request.
    GroupDmInviteRequest,
    /// Group DM invite reply.
    GroupDmInviteReply,
}

impl EnvelopeKind {
    /// Wire-format string for diagnostics, JSON storage, and dedup keying.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CallAccept => "call_accept",
            Self::CallDecline => "call_decline",
            Self::CallEnd => "call_end",
            Self::CallMediaState => "call_media_state",
            Self::CallReaction => "call_reaction",
            Self::GroupCallOffer => "group_call_offer",
            Self::GroupCallAccept => "group_call_accept",
            Self::GroupCallDecline => "group_call_decline",
            Self::FriendRequestInbox => "friend_request_inbox",
            Self::FriendAcceptInbox => "friend_accept_inbox",
            Self::DmMessage => "dm_message",
            Self::DmInviteRequest => "dm_invite_request",
            Self::DmInviteReply => "dm_invite_reply",
            Self::GroupDmInviteRequest => "group_dm_invite_request",
            Self::GroupDmInviteReply => "group_dm_invite_reply",
        }
    }

    /// Inverse of `as_str`. Returns `None` for unrecognized strings (the
    /// SQLite `envelope_kind` column may carry a stale value after a
    /// schema bump that drops a kind).
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "call_accept" => Self::CallAccept,
            "call_decline" => Self::CallDecline,
            "call_end" => Self::CallEnd,
            "call_media_state" => Self::CallMediaState,
            "call_reaction" => Self::CallReaction,
            "group_call_offer" => Self::GroupCallOffer,
            "group_call_accept" => Self::GroupCallAccept,
            "group_call_decline" => Self::GroupCallDecline,
            "friend_request_inbox" => Self::FriendRequestInbox,
            "friend_accept_inbox" => Self::FriendAcceptInbox,
            "dm_message" => Self::DmMessage,
            "dm_invite_request" => Self::DmInviteRequest,
            "dm_invite_reply" => Self::DmInviteReply,
            "group_dm_invite_request" => Self::GroupDmInviteRequest,
            "group_dm_invite_reply" => Self::GroupDmInviteReply,
            _ => return None,
        })
    }

    /// True if this kind is part of an expect-reply pair — the queue
    /// returns a oneshot to the caller and waits for a correlated reply.
    pub fn is_request(self) -> bool {
        matches!(self, Self::DmInviteRequest | Self::GroupDmInviteRequest)
    }

    /// True if this kind is an expected reply for a request.
    pub fn is_reply(self) -> bool {
        matches!(self, Self::DmInviteReply | Self::GroupDmInviteReply)
    }

    /// Map this kind to its wire `TypeId` for `Sender::send_dm` framing.
    /// Returns `None` for kinds that don't flow through `app_message`
    /// (e.g. `FriendRequestInbox` / `FriendAcceptInbox` are DHT writes;
    /// the queue refuses to enqueue them and W16.10 routes them through
    /// `operations::friend` directly).
    pub fn wire_type_id(self) -> Option<TypeId> {
        Some(match self {
            // 1:1 call signaling — post-decision phase (W16 + W16.5b)
            Self::CallAccept => TypeId::CallAccept,
            Self::CallDecline => TypeId::CallDecline,
            Self::CallEnd => TypeId::CallEnd,
            Self::CallMediaState => TypeId::CallMediaState,
            Self::CallReaction => TypeId::CallReaction,
            // Group calls (W16 — fire-and-forget)
            Self::GroupCallOffer => TypeId::GroupCallOffer,
            Self::GroupCallAccept => TypeId::GroupCallAccept,
            Self::GroupCallDecline => TypeId::GroupCallDecline,
            // DM invites (W16 — expect-reply)
            Self::DmInviteRequest => TypeId::DmInviteRequest,
            Self::DmInviteReply => TypeId::DmInviteReply,
            Self::GroupDmInviteRequest => TypeId::GroupDmInviteRequest,
            Self::GroupDmInviteReply => TypeId::GroupDmInviteReply,
            // DM body — Signal-encrypted
            Self::DmMessage => TypeId::DmMessage,
            // 3-phase friend-add inbox writes — DHT writes, no app_message TypeId.
            // The queue refuses these; W16.10 routes through operations::friend.
            Self::FriendRequestInbox | Self::FriendAcceptInbox => return None,
        })
    }
}

/// A row in the pending-envelopes table. Persisted by `EnvelopeStore`,
/// consumed by `envelope_queue::run_retry_tick`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEnvelope {
    /// Stable id assigned by the store on enqueue. Used by mark_delivered /
    /// mark_retry / mark_dead.
    pub id: i64,
    /// Sender pubkey (hex Ed25519) — used to scope queries.
    pub owner_key: String,
    /// Recipient pubkey (hex Ed25519). Wire envelope routes via the
    /// peer's cached private route blob.
    pub recipient_key: String,
    /// Discriminator. Drives retry config + send-shape choice.
    pub kind: EnvelopeKind,
    /// Per-recipient sequence (W16.3). Allocated via `next_outbound_seq`.
    pub seq: u64,
    /// Optional grouping: call_id for call envelopes, conversation id
    /// for DMs, request_id for expect-reply pairs.
    pub correlation_id: Option<String>,
    /// Serialized `MessagePayload` (postcard via the existing transport
    /// codec). Opaque to the store.
    pub payload: Vec<u8>,
    /// Wall-clock ms when the row was first enqueued.
    pub created_at_ms: u64,
    /// Wall-clock ms when this row becomes eligible for the next attempt.
    /// Initially equal to `created_at_ms`.
    pub next_retry_at_ms: u64,
    /// Number of failed attempts so far.
    pub retry_count: u32,
    /// Per-kind cap. Beyond this the row is dead-lettered.
    pub max_retries: u32,
    /// Last error string. Populated on retry; surfaces in the
    /// `EnvelopeDeliveryFailed` notification when the cap is hit.
    pub last_error: Option<String>,
}

/// Persisted Dialing / Incoming call state for crash recovery (W16.8).
/// Active calls are intentionally NOT persisted — voice transport state
/// (cpal stream, encoder, jitter buffer) is process-bound and cannot
/// meaningfully resume. Matches Signal RingRTC + Discord Voice Gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedCallState {
    pub owner_key: String,
    pub call_id: String,
    pub peer_pubkey: String,
    /// "audio" | "video".
    pub kind: String,
    /// "outgoing" | "incoming". Other statuses are NOT persisted.
    pub status: String,
    pub expires_at_ms: u64,
    /// Outgoing-only: our ephemeral X25519 secret used to derive call_key
    /// once the receiver replies with their X25519 pub.
    pub my_x25519_secret: Option<Vec<u8>>,
    /// Incoming-only: caller's X25519 pub (from the CallInvite).
    pub peer_x25519_pub: Option<Vec<u8>>,
    /// Group calls: list of invitee pubkeys. Empty for 1:1.
    pub group_participants: Vec<String>,
    pub inserted_at_ms: u64,
}

/// Errors the store may return. Caller maps to its domain error type.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(String),
    #[error("serialize: {0}")]
    Serialize(String),
    #[error("not found: id={0}")]
    NotFound(i64),
    #[error("other: {0}")]
    Other(String),
}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialize(value.to_string())
    }
}

/// The persistence contract every reliability-aware host implements.
///
/// Three host types in this project today:
/// - `JsonEnvelopeStore` — atomic JSON file (this crate, default for
///   rekindle-cli + rekindle-node).
/// - `MemoryEnvelopeStore` — in-process HashMap (this crate, tests).
/// - `SqliteEnvelopeStore` — provided by `src-tauri/src/envelope_store_sqlite.rs`,
///   uses the existing rusqlite pool. Lives in src-tauri because rusqlite
///   is heavy and Tauri is the only host shipping it.
///
/// The trait is intentionally minimal — every method maps 1:1 to a
/// concrete operation the queue or call runtime performs. No "generic
/// query" surface area; no leaking-storage-shape into the trait.
#[async_trait]
pub trait EnvelopeStore: Send + Sync {
    // ── Pending envelopes (W16.2) ────────────────────────────────────

    /// Persist a new envelope row. The store assigns the `id` (caller
    /// passes 0 for `id` when constructing the input).
    async fn enqueue(&self, env: PendingEnvelope) -> Result<i64, StoreError>;

    /// Read all eligible envelopes for an owner: rows where
    /// `next_retry_at_ms <= now_ms`, ordered by `next_retry_at_ms` ASC,
    /// limited to `limit` per tick.
    async fn load_eligible(
        &self,
        owner_key: &str,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<PendingEnvelope>, StoreError>;

    /// Mark an envelope as delivered — delete the row.
    async fn mark_delivered(&self, id: i64) -> Result<(), StoreError>;

    /// Reschedule an envelope for retry — bump `retry_count`,
    /// `next_retry_at_ms`, and `last_error`.
    async fn mark_retry(
        &self,
        id: i64,
        retry_count: u32,
        next_retry_at_ms: u64,
        last_error: &str,
    ) -> Result<(), StoreError>;

    /// Mark an envelope as dead-lettered — delete the row. Caller logs +
    /// emits `EnvelopeDeliveryFailed` separately.
    async fn mark_dead(&self, id: i64) -> Result<(), StoreError>;

    /// Cancel all envelopes correlated to a given id (call hangup tears
    /// down pending envelopes for that `call_id`). Returns the number of
    /// rows deleted.
    async fn cancel_by_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<usize, StoreError>;

    // ── Per-recipient seq tracking (W16.3) ───────────────────────────

    /// Allocate the next outbound seq for `(owner, recipient, kind, correlation_id)`.
    /// Increments + persists atomically.
    async fn next_outbound_seq(
        &self,
        owner_key: &str,
        recipient_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<u64, StoreError>;

    /// Record an inbound seq as seen. Caller checks `get_last_inbound_seq`
    /// first to detect duplicates.
    async fn record_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
        seq: u64,
        now_ms: u64,
    ) -> Result<(), StoreError>;

    /// Returns Some(last_seq) if `(owner, sender, kind, correlation_id)`
    /// was seen before; None if first time.
    async fn get_last_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<Option<u64>, StoreError>;

    // ── Active call state for crash recovery (W16.8) ─────────────────

    /// Persist Dialing/Incoming call state. Existing row for the same
    /// `(owner, call_id)` is replaced (UPSERT semantics).
    async fn save_active_call(&self, state: PersistedCallState) -> Result<(), StoreError>;

    /// Delete the persisted state for a given call (call ended, declined,
    /// timed out, etc.).
    async fn delete_active_call(
        &self,
        owner_key: &str,
        call_id: &str,
    ) -> Result<(), StoreError>;

    /// Load all persisted active call states for an owner — used on
    /// startup to rehydrate Dialing/Incoming UI.
    async fn load_active_calls(
        &self,
        owner_key: &str,
    ) -> Result<Vec<PersistedCallState>, StoreError>;
}
