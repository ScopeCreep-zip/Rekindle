//! Shared `MockCommunityDeps` test fixture for community-presence
//! orchestrators (poll, registry, etc).
//!
//! Decomposed per Invariant 1 (≤500 LoC per file): `state.rs`
//! holds the `MockState` struct (~80 LoC of fields); this module
//! exposes the dispatch wrapper + the `CommunityPresenceDeps`
//! trait impl. Every trait-method call records its inputs into a
//! `calls_*` Vec on `MockState` so tests assert both behaviour AND
//! the exact (community_id, …) values that flowed through.

mod state;

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::deps::{
    CommunityPresenceDeps, DiscoveredMemberRow, PresenceCredentials, PresenceError,
    SegmentDescriptor, SelfPresenceSnapshot,
};

pub use state::MockState;

pub struct MockCommunityDeps {
    pub state: Mutex<MockState>,
    /// An open session scope: the orchestrators run to completion.
    pub scope: std::sync::Arc<rekindle_lifecycle::SessionScope>,
}

impl MockCommunityDeps {
    /// A fixture over `state` inside an open session scope.
    pub fn new(state: MockState) -> Self {
        Self {
            state: Mutex::new(state),
            scope: rekindle_lifecycle::SessionScope::new("test", std::sync::Arc::new(|_| {})),
        }
    }
}

#[async_trait]
impl CommunityPresenceDeps for MockCommunityDeps {
    /// A fixed clock: the fixture's runs are reproducible.
    fn now_secs(&self) -> u64 {
        1_700_000_000
    }
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        std::sync::Arc::clone(&self.scope)
    }
    fn my_pseudonym_for_community(&self, community_id: &str) -> String {
        let mut st = self.state.lock();
        st.calls_my_pseudonym.push(community_id.to_string());
        st.my_pk.clone()
    }
    fn our_route_blob(&self) -> Option<Vec<u8>> {
        self.state.lock().our_route.clone()
    }
    /// The fixture's route is whatever its state holds: no wait.
    async fn own_route_available(&self) {}
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
        // Sized like the real MEK seal: nonce (12) + plaintext + tag (16).
        let plaintext = serde_json::to_vec(ranges).ok()?;
        Some(rekindle_types::presence::EncryptedHistoryRanges {
            mek_generation: 1,
            ciphertext: vec![0; 12 + plaintext.len() + 16],
        })
    }
    fn self_session(&self, _community_id: &str) -> rekindle_types::presence::MemberSession {
        rekindle_types::presence::MemberSession::default()
    }
    fn presence_policy(
        &self,
        _community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy {
        rekindle_types::presence::PresenceSharingPolicy::default()
    }
    fn encrypt_session_extras_with_current_mek(
        &self,
        _community_id: &str,
        _extras: &rekindle_types::presence::SessionExtras,
    ) -> Option<rekindle_types::presence::EncryptedSessionExtras> {
        None
    }
    fn decrypt_session_extras(
        &self,
        _community_id: &str,
        _encrypted: &rekindle_types::presence::EncryptedSessionExtras,
    ) -> Option<rekindle_types::presence::SessionExtras> {
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
    ) -> Result<crate::deps::RowWrite, PresenceError> {
        let mut st = self.state.lock();
        st.calls_write_registry.push((
            registry_key.to_string(),
            subkey,
            payload.len(),
            writer_keypair_str.to_string(),
        ));
        Ok(st
            .write_outcomes
            .pop_front()
            .unwrap_or(crate::deps::RowWrite::Stored))
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
    fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String> {
        self.state
            .lock()
            .calls_extend_known
            .push((community_id.to_string(), candidates.len()));
        candidates
    }
    fn emit_member_discovered(&self, community_id: &str, pseudonym_key: &str, subkey_index: u32) {
        self.state.lock().calls_emit_discovered.push((
            community_id.to_string(),
            pseudonym_key.to_string(),
            subkey_index,
        ));
    }
    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String> {
        self.state
            .lock()
            .calls_run_tick
            .push(community_id.to_string());
        Ok(())
    }
    async fn ensure_registry_open(&self, community_id: &str) -> Result<Option<String>, String> {
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
    fn apply_member_state_update(
        &self,
        community_id: &str,
        known_member_keys: HashSet<String>,
        _banned_members: &HashSet<String>,
    ) {
        self.state
            .lock()
            .calls_apply_member_state
            .push((community_id.to_string(), known_member_keys.len()));
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
    fn active_voice_channel(&self, _community_id: &str) -> Option<String> {
        self.state.lock().active_voice_channel.clone()
    }
    fn reconcile_voice_roster(&self, community_id: &str, rows: Vec<crate::deps::VoicePresenceRow>) {
        self.state
            .lock()
            .calls_reconcile_voice_roster
            .push((community_id.to_string(), rows));
    }
    fn maybe_auto_expand_segment(&self, community_id: &str) {
        self.state
            .lock()
            .calls_auto_expand
            .push(community_id.to_string());
    }
}
