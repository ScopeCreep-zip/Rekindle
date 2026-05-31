//! Phase 15.r split — `impl FilesDeps for FilesAdapter`.
//!
//! All 28 trait methods in one place. Bigger bodies delegate to
//! `helpers` so this file stays under the per-file behavior cap.

use std::path::Path;

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_files::{ChunkCache, FilesDeps, FilesError, FilesEvent, PinnedSet};
use rekindle_protocol::dht::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::attachment::AttachmentOffer;
use uuid::Uuid;

use super::helpers;
use super::FilesAdapter;
use crate::state_helpers;

#[async_trait]
impl FilesDeps for FilesAdapter {
    // ── Identity ───────────────────────────────────────────────────

    fn owner_key(&self) -> Result<String, FilesError> {
        let key = state_helpers::owner_key_or_default(&self.state);
        if key.is_empty() {
            Err(FilesError::IdentityNotLoaded)
        } else {
            Ok(key)
        }
    }

    fn my_pseudonym(&self, community_id: &str) -> Result<String, FilesError> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or_else(|| FilesError::NotFound(format!("pseudonym for {community_id}")))
    }

    // ── Channel + community state ──────────────────────────────────

    fn channel_log_key(&self, community_id: &str, channel_id: &str) -> Result<String, FilesError> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.channel_log_keys.get(channel_id).cloned())
            .ok_or_else(|| {
                FilesError::NotFound(format!("channel_log_key {community_id}/{channel_id}"))
            })
    }

    fn slot_keypair(&self, community_id: &str) -> Result<String, FilesError> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.slot_keypair.clone())
            .ok_or_else(|| FilesError::NotFound(format!("slot_keypair {community_id}")))
    }

    fn my_subkey_index(&self, community_id: &str) -> Result<u32, FilesError> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.my_subkey_index)
            .ok_or_else(|| FilesError::NotFound(format!("my_subkey_index {community_id}")))
    }

    fn channel_is_forum(&self, community_id: &str, channel_id: &str) -> bool {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return false;
        };
        community.channels.iter().any(|ch| {
            ch.id == channel_id && matches!(ch.channel_type, crate::state::ChannelType::Forum)
        })
    }

    fn mek_generation(&self, community_id: &str) -> Result<u64, FilesError> {
        self.state
            .communities
            .read()
            .get(community_id)
            .map(|c| c.mek_generation)
            .ok_or_else(|| FilesError::NotFound(format!("community {community_id}")))
    }

    fn channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<MediaEncryptionKey, FilesError> {
        let cache = self.state.channel_mek_cache.lock();
        if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
            return Ok(mek.clone());
        }
        drop(cache);
        self.state
            .mek_cache
            .lock()
            .get(community_id)
            .cloned()
            .ok_or_else(|| FilesError::MekUnavailable {
                community: community_id.to_string(),
                generation: 0,
            })
    }

    fn historical_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        helpers::historical_channel_mek_impl(&self.state, community_id, channel_id, generation)
    }

    fn community_mek(&self, community_id: &str) -> Option<MediaEncryptionKey> {
        self.state.mek_cache.lock().get(community_id).cloned()
    }

    // ── Permissions + slowmode + mentions ──────────────────────────

    fn require_permission(
        &self,
        community_id: &str,
        permission: Permissions,
    ) -> Result<(), FilesError> {
        crate::commands::community::require_permission(&self.state, community_id, permission)
            .map_err(FilesError::PermissionDenied)
    }

    fn enforce_slowmode(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), FilesError> {
        crate::services::community::channel_messages::enforce_slowmode(
            &self.state,
            community_id,
            channel_id,
            now_ms,
        )
        .map_err(FilesError::Slowmode)
    }

    fn resolve_outbound_mentions(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        body: &str,
    ) -> (Vec<String>, Vec<String>, u32) {
        crate::services::community::channel_messages::resolve_outbound_mentions(
            &self.state,
            community_id,
            sender_pseudonym,
            body,
        )
    }

    // ── Lamport + sequence ─────────────────────────────────────────

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64 {
        let mut communities = self.state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            let s = cs
                .channel_sequences
                .entry(channel_id.to_string())
                .or_insert(0);
            *s += 1;
            *s
        } else {
            1
        }
    }

    // ── Cache ──────────────────────────────────────────────────────

    fn ensure_cache_open(&self, community_id: &str) -> Result<(), FilesError> {
        crate::services::community::files::ensure_cache_open(&self.state, community_id)
            .map_err(FilesError::NotFound)
    }

    fn pinned_attachments(&self, community_id: &str) -> PinnedSet {
        self.state
            .pinned_attachments
            .read()
            .get(community_id)
            .cloned()
            .unwrap_or_default()
    }

    fn set_community_pinned_set(&self, community_id: &str, ids: Vec<uuid::Uuid>) {
        let mut all = self.state.pinned_attachments.write();
        let entry = all.entry(community_id.to_string()).or_default();
        entry.replace(ids);
    }

    fn with_cache_mut(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&mut ChunkCache, &PinnedSet) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let mut caches = self.state.file_caches.write();
        let cache = caches
            .get_mut(community_id)
            .ok_or_else(|| FilesError::NotFound(format!("file cache for {community_id}")))?;
        let pinned_lock = self.state.pinned_attachments.read();
        let pinned = pinned_lock.get(community_id).cloned().unwrap_or_default();
        f(cache, &pinned)
    }

    fn with_cache(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&ChunkCache) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let caches = self.state.file_caches.read();
        let cache = caches
            .get(community_id)
            .ok_or_else(|| FilesError::NotFound(format!("file cache for {community_id}")))?;
        f(cache)
    }

    // ── Online-peer routing (gossip overlay) ───────────────────────

    fn peer_route_blob(&self, community_id: &str, target_pseudonym: &str) -> Option<Vec<u8>> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.gossip.clone())
            .and_then(|gossip| {
                gossip
                    .online_members
                    .get(target_pseudonym)
                    .map(|m| m.route_blob.clone())
            })
    }

    fn online_member_pseudonyms(&self, community_id: &str) -> Vec<String> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.gossip.as_ref())
            .map(|g| g.online_members.keys().cloned().collect())
            .unwrap_or_default()
    }

    // ── Governance reads ───────────────────────────────────────────

    fn governance_pinned_attachments(&self, community_id: &str) -> Vec<Uuid> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.governance_state.as_ref())
            .map(|gov| {
                gov.pinned_attachments
                    .iter()
                    .map(|bytes| Uuid::from_bytes(*bytes))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn governance_expressions_with_attachments(
        &self,
        community_id: &str,
    ) -> Vec<([u8; 16], AttachmentOffer)> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.governance_state.as_ref())
            .map(|gov| {
                gov.expressions
                    .iter()
                    .filter_map(|(eid, expr)| expr.attachment.clone().map(|a| (*eid, a)))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ── Gossip ─────────────────────────────────────────────────────

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), FilesError> {
        crate::services::community::send_to_mesh(&self.state, community_id, envelope)
            .map_err(FilesError::Transport)
    }

    // ── Persistence (fire-and-forget) ──────────────────────────────

    fn persist_slowmode_state(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        helpers::persist_slowmode_state_impl(
            &self.state,
            &self.pool,
            community_id,
            channel_id,
            now_ms,
        );
    }

    // ── Events ─────────────────────────────────────────────────────

    fn emit_event(&self, event: FilesEvent) {
        let mapped = helpers::map_files_event(event);
        crate::event_dispatch::emit_live(&self.app_handle, "community-event", &mapped);
    }

    // ── DHT operations (async, real impls) ─────────────────────────

    async fn write_channel_message_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        message: &ChannelMessage,
    ) -> Result<(), FilesError> {
        helpers::write_channel_message_impl(
            &self.state,
            community_id,
            channel_log_key,
            slot_index,
            slot_keypair,
            message,
        )
        .await
    }

    async fn write_attachment_cached_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        cached: &ChannelAttachmentCached,
    ) -> Result<(), FilesError> {
        helpers::write_attachment_cached_impl(
            &self.state,
            community_id,
            channel_log_key,
            slot_index,
            slot_keypair,
            cached,
        )
        .await
    }

    async fn scan_channel_subkeys(
        &self,
        channel_log_key: &str,
    ) -> Result<Vec<ChannelRecordEntry>, FilesError> {
        let record_key = channel_log_key
            .parse::<veilid_core::RecordKey>()
            .map_err(|e| FilesError::Transport(format!("invalid channel record key: {e}")))?;
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or_else(|| FilesError::Transport("not attached".into()))?;
        let mut all = Vec::new();
        for subkey in 0u32..255 {
            let Ok(Some(value)) = rc.get_dht_value(record_key.clone(), subkey, false).await else {
                continue;
            };
            let Ok(entries) =
                rekindle_protocol::dht::community::channel_record::decode_channel_entries(
                    value.data(),
                )
            else {
                continue;
            };
            all.extend(entries);
        }
        Ok(all)
    }

    async fn app_call_peer(
        &self,
        route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, FilesError> {
        let api = state_helpers::veilid_api(&self.state)
            .ok_or_else(|| FilesError::Transport("Veilid API unavailable".into()))?;
        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| FilesError::Transport(format!("import route: {e}")))?;
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or_else(|| FilesError::Transport("not attached".into()))?;
        rc.app_call(veilid_core::Target::RouteId(route_id), payload)
            .await
            .map_err(|e| FilesError::Transport(format!("app_call: {e}")))
    }

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
    ) -> Result<(), FilesError> {
        helpers::insert_channel_message_full_impl(
            &self.pool,
            owner_key,
            channel_id,
            sender_key,
            message_id,
            timestamp_ms,
            mek_generation,
            lamport_ts,
            attachment_json,
            flags,
            body,
        )
        .await
    }

    async fn persist_local_path(
        &self,
        owner_key: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        save_path: &Path,
    ) -> Result<(), FilesError> {
        helpers::persist_local_path_impl(
            &self.pool,
            owner_key,
            channel_id,
            attachment_id_hex,
            save_path,
        )
        .await
    }

    async fn write_attachment_pinned(
        &self,
        community_id: &str,
        attachment_id: [u8; 16],
        pinned: bool,
        lamport: u64,
    ) -> Result<(), FilesError> {
        crate::services::community::write_entry(
            &self.state,
            community_id,
            rekindle_types::governance::GovernanceEntry::AttachmentPinned {
                attachment_id,
                pinned,
                lamport,
            },
        )
        .await
        .map_err(FilesError::Transport)
    }
}
