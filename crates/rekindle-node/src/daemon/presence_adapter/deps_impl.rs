//! `CommunityPresenceDeps` impl — delegation plus the recorded gaps.
//!
//! Methods that return empty do so for a checked reason, named at each
//! one — not as a placeholder. Two that look like they should be empty
//! are not: `read_all_channel_messages` is a DHT read the daemon can
//! perform perfectly well, and `active_voice_channel` is blocked by
//! missing local state rather than a missing subsystem.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rekindle_codec::community::channel_record::ChannelMessage;
use rekindle_codec::community::envelope::{CommunityEnvelope, SignedEnvelope};
use rekindle_presence::community::{
    GossipOverlayPlan, GossipOverlaySnapshot, MemberProfileSnapshot,
};
use rekindle_presence::deps::{
    CommunityPresenceDeps, DiscoveredMemberRow, OnlineMember, PresenceCredentials, PresenceError,
    SegmentDescriptor, SelfPresenceSnapshot, VoicePresenceRow,
};
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::presence::{
    EncryptedHistoryRanges, EncryptedSessionExtras, HistoryRange, MemberSession,
    PresenceSharingPolicy, SessionExtras,
};

use super::DaemonPresenceAdapter;

#[async_trait]
impl CommunityPresenceDeps for DaemonPresenceAdapter {
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        self.ctx.unlock_scope_or_closed()
    }

    // ---------- Identity and membership ----------

    fn my_pseudonym_for_community(&self, community_id: &str) -> String {
        self.my_pseudonym_impl(community_id)
    }

    fn our_route_blob(&self) -> Option<Vec<u8>> {
        self.our_route_blob_impl()
    }

    fn current_presence_status_str(&self, _community_id: &str) -> String {
        Self::current_status_impl()
    }

    fn identity_display_name(&self) -> String {
        self.identity_display_name_impl()
    }

    fn channel_ids_for_community(&self, community_id: &str) -> Vec<String> {
        self.channel_ids_impl(community_id)
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)> {
        self.channel_log_keys_impl(community_id)
    }

    async fn member_slots_for_community(&self, community_id: &str) -> Vec<u32> {
        self.member_slots_impl(community_id)
    }

    fn presence_credentials(&self, community_id: &str) -> Option<PresenceCredentials> {
        self.presence_credentials_impl(community_id)
    }

    fn segment_descriptors(&self, community_id: &str) -> Vec<SegmentDescriptor> {
        self.segment_descriptors_impl(community_id)
    }

    fn governance_bans(&self, community_id: &str) -> HashSet<String> {
        self.governance_bans_impl(community_id)
    }

    // ---------- Registry scan and roster ----------

    async fn scan_segment_raw(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        self.scan_segment_impl(registry_key, max_subkey, skip_subkey)
            .await
    }

    async fn write_presence_to_registry_subkey(
        &self,
        registry_key: &str,
        subkey_index: u32,
        presence_json: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<rekindle_presence::RowWrite, PresenceError> {
        self.write_presence_impl(
            registry_key,
            subkey_index,
            presence_json,
            writer_keypair_str,
        )
        .await
    }

    fn persist_discovered_member_rows(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned_pseudonyms: Vec<String>,
        _joined_at: i64,
    ) {
        self.persist_members_impl(community_id, rows, &banned_pseudonyms);
    }

    fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String> {
        self.extend_known_members_impl(community_id, candidates)
    }

    async fn ensure_registry_open(&self, community_id: &str) -> Result<Option<String>, String> {
        let Some(node) = self.transport() else {
            return Err("transport not started".to_string());
        };
        let Some(creds) = self.presence_credentials_impl(community_id) else {
            return Ok(None);
        };
        let Some(descriptor) = self
            .segment_descriptors_impl(community_id)
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        // The community's registry lease is the answer (plan C7.5): when
        // held, the record is open with its sticky writer. Otherwise borrow
        // it and hand the lease to the community. A raw re-open here would
        // replace the held record's writer and safety selection (V5).
        if self.ctx.community_runtime.holds_registry(community_id) {
            return Ok(Some(descriptor.registry_key));
        }
        let lease = rekindle_transport::broadcast::dht_writes::acquire_str(
            node.as_ref(),
            &descriptor.registry_key,
            creds.slot_keypair_str.as_deref(),
        )
        .await
        .map_err(|e| format!("open registry: {e}"))?;
        let merged = self.ctx.community_runtime.hold_leases(
            community_id,
            rekindle_records::lease::CommunityLeases {
                registry: Some(lease),
                ..Default::default()
            },
            |l| rekindle_transport::broadcast::dht_writes::key_of(node.as_ref(), l),
        );
        for surplus in merged.surplus {
            rekindle_transport::broadcast::dht_writes::release(node.as_ref(), surplus).await;
        }
        Ok(Some(descriptor.registry_key))
    }

    // ---------- Roles ----------

    fn read_existing_member_roles(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        self.member_roles_impl(community_id)
    }

    fn read_governance_role_assignments(
        &self,
        community_id: &str,
    ) -> HashMap<PseudonymKey, HashSet<RoleId>> {
        self.governance_role_assignments_impl(community_id)
    }

    fn read_my_role_ids(&self, community_id: &str) -> Vec<u32> {
        self.my_role_ids_impl(community_id)
    }

    fn apply_member_state_update(
        &self,
        community_id: &str,
        merged_member_roles: HashMap<String, Vec<u32>>,
        _known_member_keys: HashSet<String>,
        _banned_members: &HashSet<String>,
    ) {
        // Only the role map is stored: the known-member set and ban list
        // are already covered — the roster is replaced wholesale by
        // `persist_discovered_member_rows`, and bans come from the
        // governance CRDT rather than a second local copy.
        self.ctx
            .community_runtime
            .set_member_roles(community_id, merged_member_roles);
    }

    // ---------- Gossip overlay ----------

    fn send_to_mesh(&self, community_id: &str, envelope: CommunityEnvelope) {
        self.send_to_mesh_impl(community_id, &envelope);
    }

    fn send_to_mesh_raw(&self, community_id: &str, _envelope: SignedEnvelope) {
        // The daemon's broadcast path signs its own envelopes from the
        // community pseudonym inside `broadcast::gossip`, so a
        // pre-signed one has no way in without a second signing path.
        // Nothing calls this on this track today; logging keeps it
        // visible if something starts to.
        tracing::debug!(
            community = %community_id,
            "send_to_mesh_raw: no pre-signed broadcast path on the daemon track"
        );
    }

    fn read_gossip_snapshot(&self, _community_id: &str) -> GossipOverlaySnapshot {
        // Always empty on this track: the daemon syncs on unlock via
        // `SubscriptionManager` rather than gating on
        // `needs_initial_sync`, and `BroadcastManager` sends through the
        // mesh directly, so no envelopes are held back for later.
        GossipOverlaySnapshot::default()
    }

    fn apply_gossip_rebuild_plan(&self, community_id: &str, plan: GossipOverlayPlan) {
        self.apply_gossip_plan_impl(community_id, plan);
    }

    fn extend_online_with_recent_gossip(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMember>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    ) {
        self.extend_online_impl(
            community_id,
            online_members,
            my_pseudonym,
            eviction_threshold_secs,
        );
    }

    fn gossip_offline_diff(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMember>,
        my_pseudonym: &str,
    ) -> Vec<String> {
        self.gossip_offline_diff_impl(community_id, online_members, my_pseudonym)
    }

    // ---------- Events emitted to IPC subscribers ----------

    fn emit_member_discovered(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        display_name: &str,
        _subkey_index: u32,
    ) {
        self.emit_member_discovered_impl(community_id, pseudonym_key, display_name);
    }

    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str) {
        self.emit_presence_offline_impl(community_id, pseudonym_key);
    }

    // ---------- Presence row composition ----------

    fn self_presence_snapshot(&self, _community_id: &str) -> SelfPresenceSnapshot {
        Self::self_presence_snapshot_impl()
    }

    fn self_session(&self, _community_id: &str) -> MemberSession {
        MemberSession::default()
    }

    fn presence_policy(&self, _community_id: &str) -> PresenceSharingPolicy {
        // The daemon shares liveness and nothing else. Location and
        // activity are user-consented signals and there is no user at a
        // keyboard here to consent.
        PresenceSharingPolicy::default()
    }

    fn sign_presence_row(&self, community_id: &str, signing_bytes: &[u8]) -> Option<Vec<u8>> {
        self.sign_presence_impl(community_id, signing_bytes)
    }

    fn encrypt_history_ranges_with_current_mek(
        &self,
        _community_id: &str,
        _ranges: &[HistoryRange],
    ) -> Option<EncryptedHistoryRanges> {
        // Advertising history requires a message store to have history
        // in — see `compute_history_ranges`.
        None
    }

    fn encrypt_session_extras_with_current_mek(
        &self,
        _community_id: &str,
        _extras: &SessionExtras,
    ) -> Option<EncryptedSessionExtras> {
        // Nothing to encrypt: `presence_policy` shares neither location
        // nor activity.
        None
    }

    fn decrypt_session_extras(
        &self,
        _community_id: &str,
        _encrypted: &EncryptedSessionExtras,
    ) -> Option<SessionExtras> {
        // Peers' extras are readable in principle, but the daemon has no
        // consumer for a location or activity signal — no roster UI, no
        // notifications. Decrypting to discard it would be spending a
        // MEK operation for nothing.
        None
    }

    // ---------- Capability gaps: no message store ----------

    async fn compute_history_ranges(&self, _community_id: &str) -> Vec<HistoryRange> {
        // The daemon keeps no local message log, so it can advertise no
        // history and serve no catch-up. Mutual aid degrades: peers
        // fetch from members that do. This closes when the daemon moves
        // to SMPL channel segments (2.6c).
        Vec::new()
    }

    async fn last_channel_message_timestamp(&self, _community_id: &str, _channel_id: &str) -> i64 {
        0
    }

    async fn read_channel_message_items(
        &self,
        record_key: &str,
        member_slots: &[u32],
    ) -> Result<Vec<(u32, ChannelMessage)>, PresenceError> {
        self.read_channel_record_impl(record_key, member_slots)
            .await
    }

    fn persist_channel_catchup(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _record_key: &str,
        _messages: Vec<(u32, ChannelMessage)>,
    ) {
    }

    fn mark_initial_sync_done(&self, community_id: &str) {
        self.ctx
            .community_runtime
            .mark_channel_synced(community_id, "");
    }

    fn mark_pending_sync(&self, _community_id: &str, _channel_id: &str, _attempt: u32) {}

    fn stale_pending_syncs(
        &self,
        _community_id: &str,
        _now_secs: u64,
        _stale_window_secs: u64,
        _max_attempts: u32,
    ) -> Vec<(String, u32)> {
        Vec::new()
    }

    fn update_pending_sync(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _now_secs: u64,
        _attempt: u32,
    ) {
    }

    fn prune_pending_syncs(&self, _community_id: &str, _max_attempts: u32) {}

    // ---------- Capability gaps: no profile store, no voice engine ----------

    fn read_member_profile_snapshot(
        &self,
        _community_id: &str,
    ) -> HashMap<String, MemberProfileSnapshot> {
        // Profiles ride along in the roster rows; the daemon has no
        // separate snapshot store to diff against, so it emits no
        // profile-changed events.
        HashMap::new()
    }

    fn apply_member_profile_updates(
        &self,
        _community_id: &str,
        _updates: HashMap<String, MemberProfileSnapshot>,
        _emit_refreshed: bool,
    ) {
    }

    fn active_voice_channel(&self, _community_id: &str) -> Option<String> {
        // The daemon *does* have voice IPC (`VoiceJoin` / `VoiceLeave`),
        // so this is not "no voice engine" — it is that nothing records
        // the session. `dispatch::presence::handle_voice_join` calls
        // `operations::voice::join_voice`, returns the session in its
        // IPC reply, and stores none of it; `handle_voice_leave` takes
        // `_ctx` and tears nothing down. Until that state exists there
        // is no honest answer here, and inventing one would put a
        // member in a voice channel the daemon cannot leave.
        None
    }

    fn reconcile_voice_roster(&self, _community_id: &str, _rows: Vec<VoicePresenceRow>) {}

    // ---------- Poll lifecycle ----------

    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String> {
        self.run_poll_tick_impl(community_id).await
    }

    fn maybe_auto_expand_segment(&self, community_id: &str) {
        // Expansion is a governance write, and the joiner already
        // triggers it from `claim_registry_slot` when every segment is
        // full. Doing it from the poll as well would race two
        // `SegmentAdded` entries for the same index.
        tracing::trace!(
            community = %community_id,
            "segment expansion is driven by the join path, not the poll"
        );
    }
}
