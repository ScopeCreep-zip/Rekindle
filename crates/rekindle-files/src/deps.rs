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
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use uuid::Uuid;

use crate::cache::ChunkCache;
use crate::error::FilesError;
use crate::pinned::PinnedSet;
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

/// Events emitted to the UI as Lost Cargo flows complete.
#[derive(Debug, Clone)]
pub enum FilesEvent {
    /// Architecture §28.9 — download finished, local_path written.
    AttachmentDownloaded {
        community_id: String,
        channel_id: String,
        attachment_id_hex: String,
        local_path: String,
    },
    /// Architecture §32 W15 — expression-asset chunks fully cached;
    /// picker can render.
    ExpressionAssetReady {
        community_id: String,
        expression_id_hex: String,
    },
}

/// Single deps trait for all Lost Cargo flows. Implementations
/// supply concrete AppState / DbPool / AppHandle / Veilid wiring.
#[async_trait]
pub trait FilesDeps: Send + Sync + 'static {
    // ── Identity ───────────────────────────────────────────────────

    fn owner_key(&self) -> Result<String, FilesError>;

    fn my_pseudonym(&self, community_id: &str) -> Result<String, FilesError>;

    // ── Channel + community state ──────────────────────────────────

    fn channel_log_key(&self, community_id: &str, channel_id: &str) -> Result<String, FilesError>;

    fn slot_keypair(&self, community_id: &str) -> Result<String, FilesError>;

    fn my_subkey_index(&self, community_id: &str) -> Result<u32, FilesError>;

    fn channel_is_forum(&self, community_id: &str, channel_id: &str) -> bool;

    fn mek_generation(&self, community_id: &str) -> Result<u64, FilesError>;

    fn channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<MediaEncryptionKey, FilesError>;

    /// MEK lookup for FEK unwrap: try keystore at the requested
    /// generation, then channel_mek_cache, then community mek_cache.
    /// Returns None if no matching-generation MEK is available.
    fn historical_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey>;

    /// Snapshot of the per-community MEK cache (current generation).
    /// Used by expression_fetch which keys on community-level MEK.
    fn community_mek(&self, community_id: &str) -> Option<MediaEncryptionKey>;

    // ── Permissions + slowmode + mentions ──────────────────────────

    fn require_permission(
        &self,
        community_id: &str,
        permission: Permissions,
    ) -> Result<(), FilesError>;

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

    fn increment_lamport(&self, community_id: &str) -> u64;

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64;

    // ── Cache (sync, AppState-coupled) ─────────────────────────────

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
    ) -> Result<(), FilesError>;

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

    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors message_repo::insert_channel_message_full's SQL column shape; bundling into a struct adds construction overhead at every callsite"
    )]
    async fn insert_channel_message_full(
        &self,
        owner_key: &str,
        channel_id: &str,
        sender_key: &str,
        message_id: &str,
        timestamp_ms: i64,
        mek_generation: u64,
        lamport_ts: u64,
        attachment_json: &str,
        flags: u32,
        body: &str,
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
