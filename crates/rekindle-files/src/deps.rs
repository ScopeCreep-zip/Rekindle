//! Phase 15 — FilesDeps trait + FilesEvent enum.
//!
//! Every operation the upload/download/dht_scan/expression_fetch flows
//! need against AppState, the database, the Veilid transport, or the
//! Tauri event system is abstracted behind a single trait. The src-tauri
//! `files_adapter` implements this trait against the live runtime; the
//! crate-side bodies are parameterised over `Arc<dyn FilesDeps>` so they
//! never import `veilid-core` or `tauri` directly (Invariant 2).
//!
//! Design note: trait surface is wide (~28 methods) because the Lost
//! Cargo flows touch many subsystems (cache + DHT + transport + DB +
//! governance + permission + slowmode + mentions + gossip + events).
//! Splitting into sub-traits would add construction cost; one trait per
//! Phase 13/14 pattern keeps the adapter focused.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_codec::community::envelope::CommunityEnvelope;
use uuid::Uuid;

use crate::cache::ChunkCache;
use crate::error::FilesError;
use crate::pinned::PinnedSet;
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};
pub use rekindle_types::message::ChannelPost;

/// What an attachment message's channel write did (plan E2.5): the record
/// pool's outcome, so the message's outbox row is settled by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelWrite {
    /// Our page is stored at consensus.
    Stored,
    /// The pool holds the page and re-pushes it; its settle settles the
    /// message.
    Held,
}

/// Events emitted to the UI as Lost Cargo flows complete.
#[derive(Debug, Clone)]
pub enum FilesEvent {
    /// Architecture §28.9 — a download was saved to disk.
    AttachmentDownloaded {
        community_id: String,
        channel_id: String,
        attachment_id_hex: String,
    },
    /// Architecture §32 W15 — expression-asset chunks fully cached;
    /// picker can render.
    ExpressionAssetReady {
        community_id: String,
        expression_id_hex: String,
    },
}

/// Column set for [`FilesDeps::insert_channel_message_full`]. Mirrors
/// `rekindle_db::repo::messages::channel::insert`'s SQL shape; grouping the
/// fields keeps the trait method under the argument-count budget while
/// each field stays explicit at the construction site.
pub struct InsertChannelMessage<'a> {
    pub owner_key: &'a str,
    pub channel_id: &'a str,
    pub sender_key: &'a str,
    pub message_id: &'a str,
    pub timestamp_ms: i64,
    pub mek_generation: u64,
    pub lamport_ts: u64,
    pub attachment_json: &'a str,
    pub flags: u32,
    pub body: &'a str,
}

/// The community keys and the chunk caches: all the expression store
/// (`expressions`) and the key lookups (`keys`) need, so a host that serves
/// expressions implements it alone (plan E3.1 E3c).
pub trait FileCacheDeps: Send + Sync + 'static {
    /// Community and channel keys (plan D6).
    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider>;

    fn ensure_cache_open(&self, community_id: &str) -> Result<(), FilesError>;

    /// Returns a snapshot of the pinned set so the caller can pass it
    /// into ChunkCache::insert() (which needs a borrow of the set).
    fn pinned_attachments(&self, community_id: &str) -> PinnedSet;

    /// Replace the per-community pinned attachment set. Used by
    /// `sync_pinned_from_governance` after a governance merge to mirror
    /// the canonical state into the cache's eviction-exemption set.
    fn set_community_pinned_set(&self, community_id: &str, ids: Vec<Uuid>);

    /// Borrow the cache + pinned set under a single write lock and
    /// run `f`. Used by upload / download / expression_fetch for
    /// chunk insertion (cache.insert needs &mut ChunkCache + &PinnedSet
    /// borrowed together).
    fn with_cache_mut(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&mut ChunkCache, &PinnedSet) -> Result<(), FilesError>,
    ) -> Result<(), FilesError>;

    /// Read-only cache access for bitmap/get queries.
    fn with_cache(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&ChunkCache) -> Result<(), FilesError>,
    ) -> Result<(), FilesError>;
}

/// Single deps trait for all Lost Cargo flows. Implementations
/// supply concrete AppState / Db / AppHandle / Veilid wiring.
#[async_trait]
pub trait FilesDeps: FileCacheDeps {
    // ── Identity ───────────────────────────────────────────────────

    fn owner_key(&self) -> Result<String, FilesError>;

    fn my_pseudonym(&self, community_id: &str) -> Result<String, FilesError>;

    // ── Channel + community state ──────────────────────────────────

    fn channel_log_key(&self, community_id: &str, channel_id: &str) -> Result<String, FilesError>;

    fn slot_keypair(&self, community_id: &str) -> Result<String, FilesError>;

    fn my_subkey_index(&self, community_id: &str) -> Result<u32, FilesError>;

    fn channel_is_forum(&self, community_id: &str, channel_id: &str) -> bool;

    // ── Permissions + slowmode + mentions ──────────────────────────

    fn require_permission(&self, community_id: &str, permission: u64) -> Result<(), FilesError>;

    fn enforce_slowmode(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), FilesError>;

    /// Returns (mentioned_pseudonyms, mentioned_roles, mention_flags).
    fn resolve_outbound_mentions(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        body: &str,
    ) -> (Vec<String>, Vec<String>, u32);

    // ── Lamport + sequence ─────────────────────────────────────────

    /// Next message-clock value (attachment messages).
    fn increment_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError>;
    /// Next governance-clock value (`AttachmentPinned`).
    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError>;

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64;

    // ── Online-peer routing (gossip overlay) ───────────────────────

    fn peer_route_blob(&self, community_id: &str, target_pseudonym: &str) -> Option<Vec<u8>>;

    fn online_member_pseudonyms(&self, community_id: &str) -> Vec<String>;

    // ── Governance reads (for expression eager-fetch) ──────────────

    fn governance_pinned_attachments(&self, community_id: &str) -> Vec<Uuid>;

    /// Returns (expression_id, offer) pairs for expressions whose
    /// attachment is not yet fully cached locally. Used by
    /// `expression_fetch::eager_fetch_missing`.
    fn governance_expressions_with_attachments(
        &self,
        community_id: &str,
    ) -> Vec<([u8; 16], AttachmentOffer)>;

    // ── DHT operations (async, veilid-core abstracted away) ────────

    /// Write a ChannelMessage entry to the channel SMPL record.
    /// Adapter handles slot_keypair string → KeyPair parsing,
    /// pseudonym credentials lookup, RecordKey construction.
    async fn write_channel_message_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        message: &ChannelMessage,
    ) -> Result<ChannelWrite, FilesError>;

    /// Store `post`'s outbox `channel_post` row, its echo the attachment
    /// message already written `queued` (plan E2.5).
    async fn queue_post(&self, post: &ChannelPost) -> Result<(), FilesError>;
    /// Our page of `record_key`/`subkey` was stored: every post waiting on
    /// it settles `sent`.
    async fn settle_posts(&self, record_key: &str, subkey: u32);
    /// `message_id`'s write could not be made: its row settles `failed`.
    async fn fail_post(&self, message_id: &str);

    async fn write_attachment_cached_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        cached: &ChannelAttachmentCached,
    ) -> Result<(), FilesError>;

    /// Scan all 255 member subkeys of a channel SMPL record and
    /// return the decoded entries in arrival order. Used by
    /// `fetch_attachment_offer` (looks for Message entries with
    /// matching attachment_id) and `discover_sources` (looks for
    /// AttachmentCached entries).
    async fn scan_channel_subkeys(
        &self,
        channel_log_key: &str,
    ) -> Result<Vec<ChannelRecordEntry>, FilesError>;

    /// Make an app_call to a peer over their imported private route,
    /// returning the reply bytes. Adapter handles route blob import +
    /// RoutingContext lookup.
    async fn app_call_peer(
        &self,
        route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, FilesError>;

    // ── Persistence (async) ────────────────────────────────────────

    async fn insert_channel_message_full(
        &self,
        row: InsertChannelMessage<'_>,
    ) -> Result<(), FilesError>;

    /// Fire-and-forget — update channel_slowmode_state row.
    fn persist_slowmode_state(&self, community_id: &str, channel_id: &str, now_ms: i64);

    /// Update messages.attachment_json.local_path for the row carrying
    /// the given attachment_id after a download completes.
    async fn persist_local_path(
        &self,
        owner_key: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        save_path: &Path,
    ) -> Result<(), FilesError>;

    // ── Gossip ─────────────────────────────────────────────────────

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), FilesError>;

    // ── Pin governance ─────────────────────────────────────────────

    async fn write_attachment_pinned(
        &self,
        community_id: &str,
        attachment_id: [u8; 16],
        pinned: bool,
        lamport: u64,
    ) -> Result<(), FilesError>;

    // ── Events ─────────────────────────────────────────────────────

    fn emit_event(&self, event: FilesEvent);
}

/// Convenience type alias used by the crate-side bodies.
pub type SharedFilesDeps = Arc<dyn FilesDeps>;

/// Helper: pass a download bitmap snapshot through `with_cache`. The
/// closure form means callers can't mistakenly hold the lock across
/// `.await`.
pub fn read_bitmap_for(
    deps: &dyn FilesDeps,
    community_id: &str,
    attachment_uuid: Uuid,
    chunk_count: u32,
) -> Result<AttachmentBitmap, FilesError> {
    let mut out: Option<AttachmentBitmap> = None;
    deps.with_cache(community_id, &mut |cache| {
        out = Some(
            cache
                .bitmap_for(attachment_uuid, chunk_count)
                .map_err(|e| FilesError::Db(format!("bitmap_for: {e}")))?,
        );
        Ok(())
    })?;
    out.ok_or_else(|| FilesError::NotFound(format!("cache for {community_id}")))
}
