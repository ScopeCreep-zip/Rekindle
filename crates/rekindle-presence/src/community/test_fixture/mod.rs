//! Shared `MockCommunityDeps` test fixture for community-presence
//! orchestrators (sync, poll, etc).
//!
//! Decomposed per Invariant 1 (≤500 LoC per file): `state.rs`
//! holds the `MockState` struct (~80 LoC of fields); this module
//! exposes the dispatch wrapper + the `CommunityPresenceDeps`
//! trait impl. Every trait-method call records its inputs into a
//! `calls_*` Vec on `MockState` so tests assert both behaviour AND
//! the exact (community_id, …) values that flowed through.

#![cfg(test)]

mod state;

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};

use crate::community::{GossipOverlayPlan, GossipOverlaySnapshot};
use crate::deps::{
    CommunityPresenceDeps, DiscoveredMemberRow, OnlineMemberSnapshot, PresenceCredentials,
    PresenceError, SegmentDescriptor, SelfPresenceSnapshot,
};

pub use state::MockState;

pub struct MockCommunityDeps {
    pub state: Mutex<MockState>,
}

#[async_trait]
impl CommunityPresenceDeps for MockCommunityDeps {
    fn my_pseudonym_for_community(&self, community_id: &str) -> String {
        let mut st = self.state.lock();
        st.calls_my_pseudonym.push(community_id.to_string());
        st.my_pk.clone()
    }
    fn our_route_blob(&self) -> Option<Vec<u8>> {
        self.state.lock().our_route.clone()
    }
    fn current_presence_status_str(&self, community_id: &str) -> String {
        let mut st = self.state.lock();
        st.calls_status_str.push(community_id.to_string());
        st.status.clone()
    }
    fn channel_ids_for_community(&self, community_id: &str) -> Vec<String> {
        let mut st = self.state.lock();
        st.calls_channel_ids.push(community_id.to_string());
        st.channels.clone()
    }
    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)> {
        let mut st = self.state.lock();
        st.calls_channel_logs.push(community_id.to_string());
        st.channel_logs.clone()
    }
    fn member_count_for_community(&self, community_id: &str) -> u32 {
        let mut st = self.state.lock();
        st.calls_member_count.push(community_id.to_string());
        st.member_count
    }
    fn send_to_mesh(&self, community_id: &str, envelope: CommunityEnvelope) {
        self.state
            .lock()
            .sent_envelopes
            .push((community_id.to_string(), envelope));
    }
    async fn last_channel_message_timestamp(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> i64 {
        let mut st = self.state.lock();
        st.calls_last_ts
            .push((community_id.to_string(), channel_id.to_string()));
        st.last_ts.get(channel_id).copied().unwrap_or(0)
    }
    fn mark_pending_sync(&self, community_id: &str, channel_id: &str, attempt: u32) {
        self.state.lock().pending_syncs.push((
            community_id.to_string(),
            channel_id.to_string(),
            attempt,
        ));
    }
    async fn read_all_channel_messages(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelMessage>, PresenceError> {
        let mut st = self.state.lock();
        st.calls_read_all
            .push((record_key.to_string(), member_count));
        st.read_results
            .get(record_key)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
            .map_err(PresenceError::Dht)
    }
    fn persist_channel_catchup(
        &self,
        community_id: &str,
        channel_id: &str,
        messages: Vec<ChannelMessage>,
    ) {
        self.state.lock().catchups.push((
            community_id.to_string(),
            channel_id.to_string(),
            messages.len(),
        ));
    }
    fn mark_initial_sync_done(&self, community_id: &str) {
        self.state.lock().initial_done.push(community_id.to_string());
    }
    fn identity_display_name(&self) -> String {
        "mock-user".to_string()
    }
    fn self_presence_snapshot(&self, community_id: &str) -> SelfPresenceSnapshot {
        self.state
            .lock()
            .calls_self_snapshot
            .push(community_id.to_string());
        SelfPresenceSnapshot::default()
    }
    fn encrypt_history_ranges_with_current_mek(
        &self,
        community_id: &str,
        ranges: &[rekindle_types::presence::HistoryRange],
    ) -> Option<rekindle_types::presence::EncryptedHistoryRanges> {
        self.state
            .lock()
            .calls_encrypt_history
            .push((community_id.to_string(), ranges.len()));
        None
    }
    async fn compute_history_ranges(
        &self,
        community_id: &str,
    ) -> Vec<rekindle_types::presence::HistoryRange> {
        self.state
            .lock()
            .calls_compute_history
            .push(community_id.to_string());
        Vec::new()
    }
    fn sign_presence_row(&self, community_id: &str, signing_bytes: &[u8]) -> Option<Vec<u8>> {
        self.state
            .lock()
            .calls_sign_presence
            .push((community_id.to_string(), signing_bytes.len()));
        Some(vec![0u8; 64])
    }
    async fn write_presence_to_registry_subkey(
        &self,
        registry_key: &str,
        subkey: u32,
        payload: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<(), PresenceError> {
        self.state.lock().calls_write_registry.push((
            registry_key.to_string(),
            subkey,
            payload.len(),
            writer_keypair_str.to_string(),
        ));
        Ok(())
    }
    fn persist_discovered_member_rows(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned: Vec<String>,
        joined_at: i64,
    ) {
        self.state.lock().calls_persist_rows.push((
            community_id.to_string(),
            rows.len(),
            banned.len(),
            joined_at,
        ));
    }
    fn extend_known_members(
        &self,
        community_id: &str,
        candidates: Vec<String>,
    ) -> Vec<String> {
        self.state
            .lock()
            .calls_extend_known
            .push((community_id.to_string(), candidates.len()));
        candidates
    }
    fn emit_member_discovered(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        display_name: &str,
        subkey_index: u32,
    ) {
        self.state.lock().calls_emit_discovered.push((
            community_id.to_string(),
            pseudonym_key.to_string(),
            display_name.to_string(),
            subkey_index,
        ));
    }
    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String> {
        self.state.lock().calls_run_tick.push(community_id.to_string());
        Ok(())
    }
    fn install_presence_poll_shutdown(
        &self,
        community_id: &str,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
    ) {
        drop(shutdown_tx);
        self.state
            .lock()
            .calls_install_shutdown
            .push(community_id.to_string());
    }
    async fn ensure_registry_open(
        &self,
        community_id: &str,
    ) -> Result<Option<String>, String> {
        let mut st = self.state.lock();
        st.calls_ensure_registry.push(community_id.to_string());
        Ok(st.registry_open_result.clone())
    }
    fn presence_credentials(&self, community_id: &str) -> Option<PresenceCredentials> {
        let mut st = self.state.lock();
        st.calls_presence_credentials.push(community_id.to_string());
        st.presence_credentials.clone()
    }
    fn governance_bans(&self, community_id: &str) -> HashSet<String> {
        let mut st = self.state.lock();
        st.calls_governance_bans.push(community_id.to_string());
        st.bans.clone()
    }
    fn segment_descriptors(&self, community_id: &str) -> Vec<SegmentDescriptor> {
        let mut st = self.state.lock();
        st.calls_segment_descriptors.push(community_id.to_string());
        st.segments.clone()
    }
    async fn scan_segment_raw(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        let mut st = self.state.lock();
        st.calls_scan_segment.push((
            String::new(),
            max_subkey,
            registry_key.to_string(),
            skip_subkey,
        ));
        st.segment_raw
            .get(registry_key)
            .cloned()
            .unwrap_or_default()
    }
    fn read_existing_member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        let mut st = self.state.lock();
        st.calls_merge_roles
            .push((community_id.to_string(), 0, String::new()));
        st.member_roles.clone()
    }
    fn read_governance_role_assignments(
        &self,
        _community_id: &str,
    ) -> HashMap<rekindle_types::id::PseudonymKey, HashSet<rekindle_types::id::RoleId>> {
        HashMap::new()
    }
    fn read_my_role_ids(&self, _community_id: &str) -> Vec<u32> {
        Vec::new()
    }
    fn apply_member_state_update(
        &self,
        community_id: &str,
        merged_member_roles: HashMap<String, Vec<u32>>,
        known_member_keys: HashSet<String>,
        _banned_members: &HashSet<String>,
    ) {
        self.state.lock().calls_apply_member_state.push((
            community_id.to_string(),
            merged_member_roles.len(),
            known_member_keys.len(),
        ));
    }
    async fn load_known_event_ids(&self, community_id: &str) -> Vec<String> {
        self.state
            .lock()
            .calls_load_known_events
            .push(community_id.to_string());
        Vec::new()
    }
    fn read_my_event_rsvps(&self, community_id: &str) -> HashMap<String, String> {
        self.state
            .lock()
            .calls_read_my_rsvps
            .push(community_id.to_string());
        HashMap::new()
    }
    fn write_event_rsvps_by_event(
        &self,
        community_id: &str,
        aggregated: HashMap<String, Vec<crate::community::EventRsvpEntry>>,
    ) {
        self.state
            .lock()
            .calls_write_rsvps
            .push((community_id.to_string(), aggregated.len()));
    }
    fn read_member_profile_snapshot(
        &self,
        community_id: &str,
    ) -> HashMap<String, crate::community::MemberProfileSnapshot> {
        self.state
            .lock()
            .calls_read_profiles
            .push(community_id.to_string());
        HashMap::new()
    }
    fn apply_member_profile_updates(
        &self,
        community_id: &str,
        updates: HashMap<String, crate::community::MemberProfileSnapshot>,
        emit_refreshed: bool,
    ) {
        self.state.lock().calls_apply_profiles.push((
            community_id.to_string(),
            updates.len(),
            emit_refreshed,
        ));
    }
    fn extend_online_with_recent_gossip(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    ) {
        let st = self.state.lock();
        let prior_len = online_members.len();
        for (key, snapshot) in &st.inject_online {
            online_members
                .entry(key.clone())
                .or_insert_with(|| snapshot.clone());
        }
        drop(st);
        self.state.lock().calls_extend_online.push((
            community_id.to_string(),
            prior_len,
            my_pseudonym.to_string(),
            eviction_threshold_secs,
        ));
    }
    fn gossip_offline_diff(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMemberSnapshot>,
        my_pseudonym: &str,
    ) -> Vec<String> {
        let mut st = self.state.lock();
        st.calls_offline_diff.push((
            community_id.to_string(),
            online_members.len(),
            my_pseudonym.to_string(),
        ));
        st.offline_diff.clone()
    }
    fn read_gossip_snapshot(&self, community_id: &str) -> GossipOverlaySnapshot {
        let mut st = self.state.lock();
        st.calls_read_gossip.push(community_id.to_string());
        st.gossip_snapshot.clone()
    }
    fn apply_gossip_rebuild_plan(&self, community_id: &str, plan: GossipOverlayPlan) {
        self.state.lock().calls_apply_gossip.push((
            community_id.to_string(),
            plan.peers.len(),
            plan.online_members.len(),
        ));
    }
    fn send_to_mesh_raw(&self, community_id: &str, envelope: SignedEnvelope) {
        self.state
            .lock()
            .calls_send_raw
            .push((community_id.to_string(), envelope));
    }
    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str) {
        self.state
            .lock()
            .calls_emit_offline
            .push((community_id.to_string(), pseudonym_key.to_string()));
    }
    fn stale_pending_syncs(
        &self,
        community_id: &str,
        now_secs: u64,
        stale_window_secs: u64,
        max_attempts: u32,
    ) -> Vec<(String, u32)> {
        let mut st = self.state.lock();
        st.calls_stale_syncs.push((
            community_id.to_string(),
            now_secs,
            stale_window_secs,
            max_attempts,
        ));
        st.stale_syncs.clone()
    }
    fn update_pending_sync(
        &self,
        community_id: &str,
        channel_id: &str,
        now_secs: u64,
        attempt: u32,
    ) {
        self.state.lock().calls_update_pending.push((
            community_id.to_string(),
            channel_id.to_string(),
            now_secs,
            attempt,
        ));
    }
    fn prune_pending_syncs(&self, community_id: &str, max_attempts: u32) {
        self.state
            .lock()
            .calls_prune_pending
            .push((community_id.to_string(), max_attempts));
    }
    fn maybe_auto_expand_segment(&self, community_id: &str) {
        self.state
            .lock()
            .calls_auto_expand
            .push(community_id.to_string());
    }
}
