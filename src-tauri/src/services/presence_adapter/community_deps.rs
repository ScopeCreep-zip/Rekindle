//! Phase 21 REDO — `CommunityPresenceDeps` impl for `PresenceAdapter`.
//!
//! Owns the 21-method community-presence surface: per-community
//! state reads (pseudonym key, route blob, channel list), DHT
//! orchestration (registry write, channel-log catch-up read), DB
//! persistence (history range computation, member upsert), MEK
//! encrypt + W26 signature, and community event emission.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rekindle_presence::{
    CommunityPresenceDeps, DiscoveredMemberRow, GossipOverlayPlan, GossipOverlaySnapshot,
    OnlineMemberSnapshot, PresenceCredentials, PresenceError, SegmentDescriptor,
    SelfPresenceSnapshot,
};
use rekindle_protocol::dht::community::channel_record::{self, ChannelMessage};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};

use crate::services::presence_adapter::PresenceAdapter;
use crate::state_helpers;

#[async_trait]
impl CommunityPresenceDeps for PresenceAdapter {
    fn my_pseudonym_for_community(&self, community_id: &str) -> String {
        super::state_reads::my_pseudonym_for_community(&self.state, community_id)
    }

    fn our_route_blob(&self) -> Option<Vec<u8>> {
        state_helpers::our_route_blob(&self.state)
    }

    fn current_presence_status_str(&self, community_id: &str) -> String {
        // Delegate to the src-tauri helper so the wire-string
        // mapping lives in exactly one place. The helper is the
        // wire-up point for per-community `MemberPresence.custom_status`
        // (architecture spec line 754); a future commit can resolve
        // `community.my_custom_status` first inside the helper and
        // the adapter automatically picks it up.
        crate::services::community::current_presence_status(&self.state, community_id).to_string()
    }

    fn channel_ids_for_community(&self, community_id: &str) -> Vec<String> {
        super::state_reads::channel_ids_for_community(&self.state, community_id)
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)> {
        super::state_reads::channel_log_keys_for_community(&self.state, community_id)
    }

    fn member_count_for_community(&self, community_id: &str) -> u32 {
        super::state_reads::member_count_for_community(&self.state, community_id)
    }

    fn send_to_mesh(&self, community_id: &str, envelope: CommunityEnvelope) {
        if let Err(error) =
            crate::services::community::send_to_mesh(&self.state, community_id, &envelope)
        {
            tracing::debug!(
                community = %community_id,
                %error,
                "send_to_mesh from community-presence deps failed",
            );
        }
    }

    async fn last_channel_message_timestamp(&self, _community_id: &str, channel_id: &str) -> i64 {
        let owner_key = state_helpers::current_owner_key(&self.state).unwrap_or_default();
        let ch = channel_id.to_string();
        crate::db_helpers::db_call(&self.pool, move |conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(timestamp), 0) FROM messages \
                 WHERE owner_key=? AND conversation_id=? AND conversation_type='channel'",
                rusqlite::params![owner_key, ch],
                |r| r.get(0),
            )
        })
        .await
        .unwrap_or(0)
    }

    fn mark_pending_sync(&self, community_id: &str, channel_id: &str, attempt: u32) {
        super::state_reads::mark_pending_sync(&self.state, community_id, channel_id, attempt);
    }

    async fn read_all_channel_messages(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelMessage>, PresenceError> {
        let rc =
            state_helpers::safe_routing_context(&self.state).ok_or(PresenceError::NotAttached)?;
        channel_record::read_all_channel_messages(&rc, record_key, member_count)
            .await
            .map_err(|e| PresenceError::Dht(e.to_string()))
    }

    fn persist_channel_catchup(
        &self,
        _community_id: &str,
        channel_id: &str,
        messages: Vec<ChannelMessage>,
    ) {
        super::persist::insert_channel_catchup_messages(
            &self.state,
            &self.pool,
            channel_id,
            messages,
        );
    }

    fn mark_initial_sync_done(&self, community_id: &str) {
        super::state_reads::mark_initial_sync_done(&self.state, community_id);
    }

    fn identity_display_name(&self) -> String {
        state_helpers::identity_display_name(&self.state)
    }

    fn self_presence_snapshot(&self, community_id: &str) -> SelfPresenceSnapshot {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return SelfPresenceSnapshot::default();
        };
        let event_rsvps = community
            .my_event_rsvps
            .iter()
            .map(|(event_id, status)| {
                let hash = blake3::hash(event_id.as_bytes());
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&hash.as_bytes()[..16]);
                rekindle_types::presence::EventRSVP {
                    event_id: rekindle_types::id::EventId(bytes),
                    status: status.clone(),
                }
            })
            .collect();
        SelfPresenceSnapshot {
            event_rsvps,
            bio: community.my_bio.clone(),
            pronouns: community.my_pronouns.clone(),
            theme_color: community.my_theme_color,
            badges: community.my_badges.clone(),
            avatar_ref: community.my_avatar_ref.clone(),
            banner_ref: community.my_banner_ref.clone(),
        }
    }

    fn encrypt_history_ranges_with_current_mek(
        &self,
        community_id: &str,
        ranges: &[rekindle_types::presence::HistoryRange],
    ) -> Option<rekindle_types::presence::EncryptedHistoryRanges> {
        let mek = {
            let cache = self.state.mek_cache.lock();
            cache.get(community_id).cloned()?
        };
        let plaintext = serde_json::to_vec(ranges).ok()?;
        let ciphertext = mek.encrypt(&plaintext).ok()?;
        Some(rekindle_types::presence::EncryptedHistoryRanges {
            mek_generation: mek.generation(),
            ciphertext,
        })
    }

    async fn compute_history_ranges(
        &self,
        community_id: &str,
    ) -> Vec<rekindle_types::presence::HistoryRange> {
        super::persist::compute_history_ranges(&self.state, &self.pool, community_id).await
    }

    fn sign_presence_row(&self, community_id: &str, signing_bytes: &[u8]) -> Option<Vec<u8>> {
        let (_, signing_key) =
            state_helpers::pseudonym_credentials(&self.state, community_id).ok()?;
        let sig = rekindle_secrets::derive::sign_with_pseudonym(&signing_key, signing_bytes);
        Some(sig.to_vec())
    }

    async fn write_presence_to_registry_subkey(
        &self,
        registry_key: &str,
        subkey_index: u32,
        presence_json: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<(), PresenceError> {
        let rc =
            state_helpers::safe_routing_context(&self.state).ok_or(PresenceError::NotAttached)?;
        let writer_kp = writer_keypair_str
            .parse::<veilid_core::KeyPair>()
            .map_err(|e| PresenceError::InvalidDhtKey(format!("writer keypair: {e}")))?;
        let reg_key = registry_key
            .parse::<veilid_core::RecordKey>()
            .map_err(|e| PresenceError::InvalidDhtKey(format!("registry key: {e}")))?;
        let write_opts = veilid_core::SetDHTValueOptions {
            writer: Some(writer_kp),
            ..Default::default()
        };
        rc.set_dht_value(reg_key, subkey_index, presence_json, Some(write_opts))
            .await
            .map_err(|e| PresenceError::Dht(e.to_string()))?;
        Ok(())
    }

    fn persist_discovered_member_rows(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned_pseudonyms: Vec<String>,
        joined_at: i64,
    ) {
        super::persist::upsert_discovered_member_rows(
            &self.state,
            &self.pool,
            community_id,
            rows,
            banned_pseudonyms,
            joined_at,
        );
    }

    fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String> {
        let mut communities = self.state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|key| cs.known_members.insert(key.clone()))
            .collect()
    }

    fn emit_member_discovered(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        display_name: &str,
        subkey_index: u32,
    ) {
        crate::event_dispatch::emit_live(
            &self.app_handle,
            "community-event",
            &crate::channels::CommunityEvent::MemberDiscovered {
                community_id: community_id.to_string(),
                pseudonym_key: pseudonym_key.to_string(),
                display_name: display_name.to_string(),
                subkey_index,
            },
        );
    }

    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String> {
        // Delegates to the crate's `presence_poll_tick` orchestrator
        // (21.i-REDO landed). The cadence loop in `spawn.rs` invokes
        // this from each timer tick; the adapter wraps `self` in
        // an Arc so the trait's `<D: ?Sized>` bound is satisfied.
        let adapter = std::sync::Arc::new(
            super::build_adapter(&self.state).ok_or_else(|| "adapter unavailable".to_string())?,
        );
        rekindle_presence::presence_poll_tick(adapter, community_id).await
    }

    fn install_presence_poll_shutdown(
        &self,
        community_id: &str,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
    ) {
        let mut communities = self.state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.presence_poll_shutdown_tx = Some(shutdown_tx);
        }
    }

    // -- presence_poll_tick surface (21.i-REDO) --

    async fn ensure_registry_open(&self, community_id: &str) -> Result<Option<String>, String> {
        super::state_reads::ensure_registry_open(&self.state, community_id).await
    }

    fn presence_credentials(&self, community_id: &str) -> Option<PresenceCredentials> {
        super::state_reads::presence_credentials(&self.state, community_id)
    }

    fn governance_bans(&self, community_id: &str) -> HashSet<String> {
        super::state_reads::governance_bans(&self.state, community_id)
    }

    fn segment_descriptors(&self, community_id: &str) -> Vec<SegmentDescriptor> {
        super::state_reads::segment_descriptors(&self.state, community_id)
    }

    async fn scan_segment_raw(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        super::scan::scan_segment_raw(&self.state, registry_key, max_subkey, skip_subkey).await
    }

    fn read_existing_member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        super::member_state::read_existing_member_roles(&self.state, community_id)
    }

    fn read_governance_role_assignments(
        &self,
        community_id: &str,
    ) -> HashMap<rekindle_types::id::PseudonymKey, HashSet<rekindle_types::id::RoleId>> {
        super::member_state::read_governance_role_assignments(&self.state, community_id)
    }

    fn read_my_role_ids(&self, community_id: &str) -> Vec<u32> {
        super::member_state::read_my_role_ids(&self.state, community_id)
    }

    fn apply_member_state_update(
        &self,
        community_id: &str,
        merged_member_roles: HashMap<String, Vec<u32>>,
        known_member_keys: HashSet<String>,
        banned_members: &HashSet<String>,
    ) {
        super::member_state::apply_member_state_update(
            &self.state,
            community_id,
            merged_member_roles,
            known_member_keys,
            banned_members,
        );
    }

    async fn load_known_event_ids(&self, community_id: &str) -> Vec<String> {
        super::member_state::load_known_event_ids(&self.state, &self.pool, community_id).await
    }

    fn read_my_event_rsvps(&self, community_id: &str) -> HashMap<String, String> {
        super::member_state::read_my_event_rsvps(&self.state, community_id)
    }

    fn write_event_rsvps_by_event(
        &self,
        community_id: &str,
        aggregated: HashMap<String, Vec<rekindle_presence::EventRsvpEntry>>,
    ) {
        super::member_state::write_event_rsvps_by_event(&self.state, community_id, aggregated);
    }

    fn read_member_profile_snapshot(
        &self,
        community_id: &str,
    ) -> HashMap<String, rekindle_presence::MemberProfileSnapshot> {
        super::member_state::read_member_profile_snapshot(&self.state, community_id)
    }

    fn apply_member_profile_updates(
        &self,
        community_id: &str,
        updates: HashMap<String, rekindle_presence::MemberProfileSnapshot>,
        emit_refreshed: bool,
    ) {
        super::member_state::apply_member_profile_updates(
            &self.state,
            &self.app_handle,
            community_id,
            updates,
            emit_refreshed,
        );
    }

    fn extend_online_with_recent_gossip(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    ) {
        super::gossip_overlay::extend_online_with_recent_gossip(
            &self.state,
            community_id,
            online_members,
            my_pseudonym,
            eviction_threshold_secs,
        );
    }

    fn gossip_offline_diff(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
    ) -> Vec<String> {
        super::gossip_overlay::gossip_offline_diff(
            &self.state,
            community_id,
            online_members,
            my_pseudonym,
        )
    }

    fn read_gossip_snapshot(&self, community_id: &str) -> GossipOverlaySnapshot {
        super::gossip_overlay::read_gossip_snapshot(&self.state, community_id)
    }

    fn apply_gossip_rebuild_plan(&self, community_id: &str, plan: GossipOverlayPlan) {
        super::gossip_overlay::apply_gossip_rebuild_plan(&self.state, community_id, plan);
    }

    fn send_to_mesh_raw(&self, community_id: &str, envelope: SignedEnvelope) {
        crate::services::community::gossip::send_to_mesh_raw(&self.state, community_id, &envelope);
    }

    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str) {
        super::gossip_overlay::emit_member_presence_offline(
            &self.state,
            community_id,
            pseudonym_key,
        );
    }

    fn stale_pending_syncs(
        &self,
        community_id: &str,
        now_secs: u64,
        stale_window_secs: u64,
        max_attempts: u32,
    ) -> Vec<(String, u32)> {
        super::pending_sync::stale_pending_syncs(
            &self.state,
            community_id,
            now_secs,
            stale_window_secs,
            max_attempts,
        )
    }

    fn update_pending_sync(
        &self,
        community_id: &str,
        channel_id: &str,
        now_secs: u64,
        attempt: u32,
    ) {
        super::pending_sync::update_pending_sync(
            &self.state,
            community_id,
            channel_id,
            now_secs,
            attempt,
        );
    }

    fn prune_pending_syncs(&self, community_id: &str, max_attempts: u32) {
        super::pending_sync::prune_pending_syncs(&self.state, community_id, max_attempts);
    }

    fn maybe_auto_expand_segment(&self, community_id: &str) {
        super::auto_expand::maybe_auto_expand_segment(&self.state, community_id);
    }
}
