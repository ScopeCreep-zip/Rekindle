//! Deps trait surface for Phase 19 channel-messaging ops (REDO).
//!
//! One composite trait (`ChannelMessagingDeps`) abstracts every
//! src-tauri/Veilid/SQLite/Stronghold capability the crate's pure
//! pipeline logic needs. The adapter at
//! `src-tauri/src/services/channel_adapter/` (task #190) implements it.
//!
//! Schwarzschild boundary: all Veilid types are exchanged as opaque
//! `String`/`Vec<u8>` here. The crate never imports `veilid-core`.
//!
//! Matches the Phase 17 `MekDistributeDeps` + Phase 18 `GovernanceRuntimeDeps`
//! pattern — single composite trait with sync state reads + async
//! DHT/I/O methods.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::channel_record::{
    ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose, ChannelPollCreate,
    ChannelPollVote, ChannelReaction, ChannelRecordEntry,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::error::ChannelError;
use crate::event::ChannelEvent;

// ---------- DTOs ----------

/// MEK material for symmetric encrypt/decrypt across the crate
/// boundary. The 32-byte raw key + generation pair lets the crate
/// reconstruct the underlying `MediaEncryptionKey` without importing
/// the cache type itself.
#[derive(Debug, Clone)]
pub struct ChannelMek {
    pub generation: u64,
    pub key_bytes: [u8; 32],
}

/// Channel-level config snapshot needed by the send pipeline
/// (slowmode + max body size + mention rules).
#[derive(Debug, Clone)]
pub struct ChannelInfoSnapshot {
    pub channel_id: String,
    pub channel_type: String,
    pub slowmode_seconds: Option<u32>,
    pub last_send_at_ms: Option<i64>,
    pub mek_generation: u64,
    pub is_forum: bool,
}

/// Outcome of a successful send — carried back to the orchestrator
/// for adapter-side DB persistence.
#[derive(Debug, Clone)]
pub struct ChannelSendOutcome {
    pub message_id: String,
    pub sender_pseudonym_hex: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub lamport_ts: u64,
    pub timestamp_ms: i64,
}

/// Where a channel-record write targets: SMPL record key + slot info.
/// Mirrors `src-tauri/services/community/channel_messages::ChannelWriteContext`.
#[derive(Debug, Clone)]
pub struct ChannelWriteContext {
    pub community_id: String,
    pub channel_id: String,
    /// SMPL record key for the channel record this writer targets.
    pub channel_key: String,
    /// String-encoded Veilid KeyPair the adapter parses back.
    pub slot_keypair_str: String,
    pub slot_index: u32,
    pub segment_index: u32,
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
}

/// One entry from a thread record DHT scan, paired with the subkey
/// it lived in (so the AAD can be reconstructed for decrypt).
#[derive(Debug, Clone)]
pub struct ChannelEntryItem {
    pub subkey_index: u32,
    pub entry: ChannelRecordEntry,
}

/// Thread metadata snapshot — wire-shape sub-set the crate needs.
#[derive(Debug, Clone)]
pub struct ThreadStateSnapshot {
    pub thread_id: String,
    pub parent_channel_id_hex: String,
    pub name: String,
    pub thread_type: String,
    pub record_key: Option<String>,
    pub invited: Vec<PseudonymKey>,
    pub forum_tag: Option<String>,
    pub auto_archive_seconds: u64,
    pub created_lamport: u64,
    pub archived_lamport: Option<u64>,
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
pub struct MemberProfileSnapshot {
    pub display_name: Option<String>,
    pub role_ids: Vec<u32>,
}

/// Role definition view used by mention validation — roles list +
/// mentionable flag per role.
#[derive(Debug, Clone)]
pub struct RoleSnapshot {
    pub id: u32,
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
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    pub creator_pseudonym: Option<String>,
    pub created_at: Option<u64>,
    pub available_to_peers: bool,
}

/// Pending write payload for the channel write retry queue (subkey +
/// envelope bytes). Mirrors `rekindle_records::retry::PendingWrite`.
#[derive(Debug, Clone)]
pub struct PendingChannelWrite {
    pub record_key: String,
    pub subkey: u32,
    pub data: Vec<u8>,
}

/// Sent-message echo for the local "you said this" UI event.
#[derive(Debug, Clone)]
pub struct SentChannelMessageEcho {
    pub message_id: String,
    pub sender_pseudonym: String,
    pub timestamp_ms: u64,
    pub body: String,
    pub channel_id: String,
}

// ---------- Trait ----------

#[async_trait]
pub trait ChannelMessagingDeps: Send + Sync {
    // ---------- Identity / credentials ----------

    fn identity_secret(&self) -> Option<[u8; 32]>;
    fn owner_key(&self) -> Option<String>;
    fn my_pseudonym_hex(&self, community_id: &str) -> Option<String>;
    fn my_role_ids(&self, community_id: &str) -> Vec<u32>;
    fn pseudonym_credentials(&self, community_id: &str)
        -> Result<PseudonymCredentials, ChannelError>;

    // ---------- Community / channel state (read) ----------

    fn channel_info(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<ChannelInfoSnapshot>;

    fn channel_write_context(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<ChannelWriteContext, ChannelError>;

    fn channel_record_key(&self, community_id: &str, channel_id: &str) -> Option<String>;

    fn community_mek(&self, community_id: &str) -> Option<ChannelMek>;
    fn channel_or_community_mek(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<ChannelMek>;
    fn current_mek_generation(&self, community_id: &str) -> Option<u64>;

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
    fn thread_state(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadStateSnapshot>;

    fn slot_seed_bytes(&self, community_id: &str) -> Option<[u8; 32]>;
    fn member_profile(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
    ) -> MemberProfileSnapshot;

    /// All known member profiles for a community, keyed by pseudonym
    /// hex. Used by mention resolve_to_wire to map display names back
    /// to pseudonyms.
    fn list_member_profiles(
        &self,
        community_id: &str,
    ) -> HashMap<String, MemberProfileSnapshot>;
    fn community_roles(&self, community_id: &str) -> Vec<RoleSnapshot>;
    fn compute_my_permissions(&self, community_id: &str) -> u64;

    // ---------- Community / channel state (mutation) ----------

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64;
    fn next_thread_sequence(&self, community_id: &str) -> u64;
    fn mark_last_send_at(&self, community_id: &str, channel_id: &str, now_ms: i64);
    fn increment_lamport(&self, community_id: &str) -> u64;
    fn track_open_records(&self, community_id: &str, record_keys: &[String]);

    // ---------- DHT ----------

    async fn write_channel_message_smpl(
        &self,
        context: &ChannelWriteContext,
        channel_msg: &ChannelMessage,
    ) -> Result<(), ChannelError>;

    async fn write_channel_forward_smpl(
        &self,
        context: &ChannelWriteContext,
        forward: &ChannelForward,
    ) -> Result<(), ChannelError>;

    async fn write_member_reaction_smpl(
        &self,
        context: &ChannelWriteContext,
        reaction: &ChannelReaction,
    ) -> Result<(), ChannelError>;

    async fn write_channel_poll_create_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollCreate,
    ) -> Result<(), ChannelError>;

    async fn write_channel_poll_vote_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollVote,
    ) -> Result<(), ChannelError>;

    async fn write_channel_poll_close_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollClose,
    ) -> Result<(), ChannelError>;

    async fn write_channel_hand_raise_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelHandRaise,
    ) -> Result<(), ChannelError>;

    /// Resolve pseudonym hex for each known stage slot subkey. The
    /// adapter walks AppState.communities (for our own slot) + DB
    /// `community_members.subkey_index` (for all known peers). Returns
    /// a map keyed by subkey index so the `list_hand_raises` reader can
    /// map subkey entries back to identifiable members.
    async fn stage_pseudonyms_by_subkey(
        &self,
        community_id: &str,
    ) -> Result<std::collections::HashMap<u32, String>, ChannelError>;

    /// Create a lazy thread SMPL record. Returns the record key the
    /// adapter persists into `community.open_community_records`.
    async fn create_smpl_thread_record(
        &self,
        slot_seed_bytes: &[u8; 32],
    ) -> Result<String, ChannelError>;

    async fn read_all_channel_entries(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelEntryItem>, ChannelError>;

    async fn read_all_channel_messages(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelMessage>, ChannelError>;

    async fn watch_community_records(
        &self,
        community_id: &str,
    ) -> Result<(), ChannelError>;

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

    // ---------- Retry queue ----------

    async fn enqueue_channel_retry(
        &self,
        pending: PendingChannelWrite,
    ) -> Result<(), ChannelError>;

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
    fn emit_delivery_succeeded(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    );
    fn emit_delivery_failed(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    );
}
