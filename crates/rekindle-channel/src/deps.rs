//! Deps trait surface for Phase 19 channel-messaging ops (REDO).
//!
//! One composite trait (`ChannelMessagingDeps`) abstracts every
//! src-tauri/Veilid/SQLite/Stronghold capability the crate's pure
//! pipeline logic needs. The Host's
//! `HostChannel` (`rekindle_node::host::adapters::channel`) implements it.
//!
//! Schwarzschild boundary: all Veilid types are exchanged as opaque
//! `String`/`Vec<u8>` here. The crate never imports `veilid-core`.
//!
//! Matches the Phase 17 `MekDistributeDeps` + Phase 18 `GovernanceRuntimeDeps`
//! pattern — single composite trait with sync state reads + async
//! DHT/I/O methods.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::error::ChannelError;
use crate::event::ChannelEvent;

// ---------- DTOs ----------

/// Channel-level config snapshot needed by the send pipeline
/// (slowmode + max body size + mention rules).
#[derive(Debug, Clone)]
pub struct ChannelInfoSnapshot {
    pub channel_id: String,
    pub channel_type: String,
    pub slowmode_seconds: Option<u32>,
    pub last_send_at_ms: Option<i64>,
    pub is_forum: bool,
}

/// Outcome of a successful send — carried back to the orchestrator
/// for adapter-side DB persistence.
#[derive(Debug, Clone)]
pub struct ChannelSendOutcome {
    pub message_id: String,
    pub sender_pseudonym_hex: String,
    /// Empty, with no generation, for a post waiting for its key (plan
    /// E3.2 S7l): it is sealed at its release.
    pub ciphertext: Vec<u8>,
    pub mek_generation: Option<u64>,
    pub timestamp_ms: i64,
    /// The message id of the message this one replies to, when it does.
    pub reply_to: Option<String>,
    /// The row's first state: `Queued` before its write, `AwaitingKey`
    /// before its channel's key.
    pub send_state: rekindle_types::message::SendState,
}

/// Pseudonym + signing key for the local user in a community.
pub struct PseudonymCredentials {
    pub pseudonym: PseudonymKey,
    pub signing_key: SigningKey,
}

/// One row from the `messages` SQLite table — used by `forward_message`
/// to copy a previously-cached body into a new channel. Only the
/// fields the forward path actually consumes are exposed.
#[derive(Debug, Clone)]
pub struct ChannelMessageRow {
    pub sender_key: String,
    pub body: String,
    /// The message id of the stored parent this row replies to.
    pub reply_to: Option<String>,
}

/// One entry of a channel's (or thread's) outbox stream (plan E3.3), read
/// from its author's channel outbox (`M`/4), verified under the author at
/// its chain seq and linked to the stream by its tag.
#[derive(Debug, Clone)]
pub struct OutboxItem {
    pub author: rekindle_types::id::PseudonymKey,
    /// The per-(author, stream) seq, frozen at creation: an author's ops
    /// are ordered by it (rules E1, R1).
    pub stream_seq: u64,
    /// The author's clock at commit, ms (signed with the entry).
    pub committed_at: u64,
    pub entry: ChannelRecordEntry,
}

/// Thread metadata snapshot — wire-shape sub-set the crate needs.
#[derive(Debug, Clone)]
pub struct ThreadStateSnapshot {
    pub thread_id: String,
    pub parent_channel_id_hex: String,
    pub name: String,
    pub thread_type: String,
    pub invited: Vec<PseudonymKey>,
    pub forum_tag: Option<String>,
    pub auto_archive_seconds: u64,
    pub created_lamport: u64,
    /// The standing manual archive's op hash (hex), if archived: a reply
    /// that names it revives the thread (plan E3.3, architecture §4.6).
    pub archived_op: Option<String>,
    pub creator_pseudonym_hex: String,
}

/// Thread info row carried back through the crate boundary for
/// list_threads / persist_thread_row. Mirrors src-tauri ThreadInfoDto.
#[derive(Debug, Clone)]
pub struct ThreadInfoSnapshot {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub forum_tag: Option<String>,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
    pub last_message_at: u64,
    pub message_count: u32,
}

/// Member-profile lookup result — display name + role ids the local
/// user holds. Used by mention-resolution + notification routing.
#[derive(Debug, Clone, Default)]
pub struct MemberMentionView {
    pub display_name: Option<String>,
    pub role_ids: Vec<rekindle_types::id::RoleId>,
}

/// Role definition view used by mention validation — roles list +
/// mentionable flag per role.
#[derive(Debug, Clone)]
pub struct RoleSnapshot {
    pub id: rekindle_types::id::RoleId,
    pub name: String,
    pub mentionable: bool,
}

/// View of a community expression (custom emoji / sticker / soundboard
/// sound) — mirrors src-tauri's `ExpressionInfo` shape. Expressions
/// persist via `GovernanceEntry::ExpressionAdded`; the inline blob
/// bytes are loaded from the Files cache by the adapter.
#[derive(Debug, Clone)]
pub struct ExpressionView {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    /// `data:` URL of the cached bytes, for a media type on the kind's
    /// allowlist (PNG/WebP/GIF images; Ogg/WebM/MP3 soundboard clips).
    pub inline_data_url: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    pub creator_pseudonym: Option<String>,
    pub created_at: Option<u64>,
    pub available_to_peers: bool,
}

/// A sent post: the payload of its outbox `channel_post` row.
pub use rekindle_types::message::ChannelPost;

/// Sent-message echo for the local "you said this" UI event.
#[derive(Debug, Clone)]
pub struct SentChannelMessageEcho {
    pub community_id: String,
    pub message_id: String,
    pub sender_pseudonym: String,
    pub timestamp_ms: u64,
    pub body: String,
    pub channel_id: String,
}

// ---------- Trait ----------

#[async_trait]
pub trait ChannelMessagingDeps: Send + Sync {
    // ---------- Clock ----------

    /// The host's clock port in seconds (plan C9.2): message, thread,
    /// poll and expression timestamps read it, never the wall clock, so
    /// the netsim drives them.
    fn now_secs(&self) -> u64;

    // ---------- Identity / credentials ----------

    fn identity_secret(&self) -> Option<[u8; 32]>;
    fn owner_key(&self) -> Option<String>;
    fn my_pseudonym_hex(&self, community_id: &str) -> Option<String>;
    fn my_role_ids(&self, community_id: &str) -> Vec<rekindle_types::id::RoleId>;
    fn pseudonym_credentials(
        &self,
        community_id: &str,
    ) -> Result<PseudonymCredentials, ChannelError>;

    // ---------- Community / channel state (read) ----------

    fn channel_info(&self, community_id: &str, channel_id: &str) -> Option<ChannelInfoSnapshot>;

    /// Community and channel keys (plan D6). Channel text, threads and
    /// attachments are under `scope_for_text`.
    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider>;

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState>;

    /// Look up the cached compiled-automod-rules for a community. The
    /// adapter holds the cache (regex JIT state is expensive); the
    /// crate-side `compiled_rules` rebuilds on fingerprint mismatch.
    fn automod_compiled_cache_get(
        &self,
        community_id: &str,
    ) -> Option<std::sync::Arc<crate::automod::AutoModCompiledCache>>;

    fn automod_compiled_cache_set(
        &self,
        community_id: &str,
        cache: std::sync::Arc<crate::automod::AutoModCompiledCache>,
    );
    fn thread_state(&self, community_id: &str, thread_id: &str) -> Option<ThreadStateSnapshot>;

    fn member_profile(&self, community_id: &str, pseudonym_hex: &str) -> MemberMentionView;

    /// All known member profiles for a community, keyed by pseudonym
    /// hex. Used by mention resolve_to_wire to map display names back
    /// to pseudonyms.
    fn list_member_profiles(&self, community_id: &str) -> HashMap<String, MemberMentionView>;
    fn community_roles(&self, community_id: &str) -> Vec<RoleSnapshot>;
    fn compute_my_permissions(&self, community_id: &str) -> u64;

    // ---------- Community / channel state (mutation) ----------

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64;
    fn next_thread_sequence(&self, community_id: &str) -> u64;
    fn mark_last_send_at(&self, community_id: &str, channel_id: &str, now_ms: i64);

    // ---------- DHT ----------

    /// End a borrow of the host's record pool.
    async fn release_record(&self, lease: LeaseId);

    /// Every entry of the stream `stream_id` (a channel of `community_id`)
    /// across its authors' channel outboxes, author by author in chain
    /// order (plan E3.3). An outbox not readable now is skipped.
    async fn read_outbox_entries(&self, community_id: &str, stream_id: &str) -> Vec<OutboxItem>;

    /// Hand records the community keeps for its session to the host,
    /// which holds and watches them (the governance runtime's hook of the
    /// same name; plan C7.5).
    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases);

    /// Plate Gate (architecture §15.4): ensure a per-segment channel
    /// SMPL record exists for the local writer before a send. Returns
    /// the record key the channel_write_context will target. No-op
    /// (returns the genesis record key) for segment-0 senders.
    /// Delegates to `rekindle_governance_runtime::ensure_channel_segment_record`.
    async fn ensure_channel_segment_record(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<String, ChannelError>;

    // ---------- The outbox's channel_post stream (plan E2.5) ----------

    /// Store `post`'s outbox row (a post with a row of its own is persisted
    /// `queued`; an op on other rows, [`PostEntry::has_row`] false, has no
    /// echo); from then on only the outbox moves the post's delivery state.
    ///
    /// [`PostEntry::has_row`]: rekindle_types::message::PostEntry::has_row
    async fn queue_post(&self, post: &ChannelPost) -> Result<(), ChannelError>;

    /// `message`'s write could not be made: its row settles `failed`.
    async fn fail_post(&self, message: &str);

    /// Store `post`'s outbox row waiting for its channel's key (the post is
    /// persisted `awaiting_key`, plan E3.2 S7l, F9-1), failing at
    /// `deadline_ms` unless the key comes first.
    async fn queue_waiting_post(
        &self,
        post: &ChannelPost,
        deadline_ms: i64,
    ) -> Result<(), ChannelError>;

    /// `community_id`'s posts waiting for their channel's key, each
    /// channel's first-composed first.
    async fn waiting_posts(&self, community_id: &str) -> Vec<ChannelPost>;

    /// Release the waiting post `message` to its write (`queued`). `false`
    /// when it was not waiting: another release took it.
    async fn release_waiting_post(&self, message: &str) -> bool;

    // ---------- DB (channel messages) ----------

    async fn persist_sent_message(
        &self,
        community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
    ) -> Result<(), ChannelError>;

    async fn persist_forwarded_message(
        &self,
        community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
        original_author_pseudonym: &str,
    ) -> Result<(), ChannelError>;

    async fn persist_channel_sequence(
        &self,
        community_id: &str,
        channel_id: &str,
        sequence: u64,
    ) -> Result<(), ChannelError>;

    async fn persist_slowmode_state(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), ChannelError>;

    async fn find_channel_message_by_id(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<ChannelMessageRow>;

    // ---------- DB (threads) ----------

    async fn persist_thread_row(
        &self,
        community_id: &str,
        thread: &ThreadInfoSnapshot,
    ) -> Result<(), ChannelError>;

    async fn load_thread_metadata(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadInfoSnapshot>;

    // ---------- Expressions (Files cache + governance state) ----------

    /// Upload an expression blob into the community's Lost Cargo cache,
    /// returning the AttachmentOffer the GovernanceEntry::ExpressionAdded
    /// references.
    fn upload_expression_to_cache(
        &self,
        community_id: &str,
        expression_id: [u8; 16],
        bytes: &[u8],
        filename: String,
        mime_type: String,
    ) -> Result<rekindle_types::attachment::AttachmentOffer, ChannelError>;

    /// Read the cached expression bytes via its `AttachmentOffer`.
    /// Returns `None` when the asset hasn't been pulled into the local
    /// cache yet (eager fetch happens on governance merge).
    fn read_expression_bytes(
        &self,
        community_id: &str,
        offer: &rekindle_types::attachment::AttachmentOffer,
    ) -> Option<Vec<u8>>;

    // ---------- Mesh ----------

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), ChannelError>;

    // ---------- Governance ----------

    /// Apply a governance entry by delegating to
    /// `rekindle_governance_runtime::apply::write_entry` through the
    /// src-tauri governance_adapter. Returns once the entry is signed,
    /// written to DHT, gossiped, and merged into the in-memory snapshot.
    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: GovernanceEntry,
    ) -> Result<(), ChannelError>;

    // ---------- Permissions ----------

    fn require_channel_permission(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        perm_bits: u64,
    ) -> Result<(), ChannelError>;

    // ---------- Events ----------

    fn emit_event(&self, event: ChannelEvent);
    fn emit_chat_event_local(&self, echo: &SentChannelMessageEcho);
    fn emit_delivery_succeeded(&self, community_id: &str, channel_id: &str, message_id: &str);
    fn emit_delivery_failed(&self, community_id: &str, channel_id: &str, message_id: &str);
    /// Our post `message_id` moved to `state` before its write: waiting
    /// for its key, or released to its write (plan E3.2 S7l).
    fn emit_send_state(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        state: rekindle_types::message::SendState,
    );
}
