//! The single `GovernanceRuntimeDeps` impl for the daemon.
//!
//! Delegation only — every body is one call into a sibling module. That
//! is the documented adapter shape (`services-pattern.md` §2) and it
//! keeps this file readable as a map of the trait surface rather than a
//! place where logic hides.

use async_trait::async_trait;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::{
    ChannelMekSnapshot, CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo,
    DiscoveredMember, GovernanceRuntimeDeps, MekSnapshot, OnlineMemberSnapshot, RecentMessageRow,
    UserStatusKind,
};
use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_governance_runtime::roles::{RoleSnapshotInsert, RoleSnapshotPatch};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::DaemonGovernanceAdapter;

#[async_trait]
impl GovernanceRuntimeDeps for DaemonGovernanceAdapter<'_> {
    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.identity_secret_impl()
    }

    fn identity_display_name(&self) -> String {
        self.identity_display_name_impl()
    }

    fn identity_status(&self) -> UserStatusKind {
        Self::identity_status_impl()
    }

    fn our_route_blob(&self) -> Vec<u8> {
        self.our_route_blob_impl()
    }

    // ---------- Community state (read) ----------

    fn community_membership(&self, community_id: &str) -> Option<CommunityMembership> {
        self.community_membership_impl(community_id)
    }

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        self.governance_state_impl(community_id)
    }

    fn online_members(&self, community_id: &str) -> Vec<OnlineMemberSnapshot> {
        self.online_members_impl(community_id)
    }

    fn open_record_keys(&self, community_id: &str) -> Vec<String> {
        self.open_record_keys_impl(community_id)
    }

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        self.set_governance_state_impl(community_id, state);
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        self.increment_lamport_impl(community_id)
    }

    fn insert_community(&self, community: CommunityInsert) {
        self.insert_community_impl(community);
    }

    fn mark_open_channel_record(&self, community_id: &str, record_key: String) {
        self.mark_open_channel_record_impl(community_id, record_key);
    }

    // ---------- DHT ----------

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        self.create_smpl_record_impl(member_pubkeys).await
    }

    async fn create_dflt_record(&self) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        self.create_dflt_record_impl().await
    }

    async fn create_overflow_record(
        &self,
        owner_keypair: String,
    ) -> Result<String, GovernanceRuntimeError> {
        self.create_overflow_record_impl(owner_keypair).await
    }

    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        Self::format_writer_keypair_impl(ed_public, ed_secret)
    }

    async fn get_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.get_dht_value_impl(record_key, subkey, force_refresh)
            .await
    }

    async fn set_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.set_dht_value_impl(record_key, subkey, value, writer)
            .await
    }

    async fn inspect_dht_record_local_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        self.inspect_local_seqs_impl(record_key).await
    }

    async fn inspect_dht_record_update_get_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        self.inspect_network_seqs_impl(record_key).await
    }

    async fn inspect_dht_record_present_subkeys(
        &self,
        record_key: &str,
    ) -> Result<Vec<u32>, GovernanceRuntimeError> {
        self.inspect_present_subkeys_impl(record_key).await
    }

    async fn open_dht_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.open_dht_record_impl(record_key, writer).await
    }

    // ---------- MEK cache ----------

    fn community_mek(&self, community_id: &str) -> Option<MekSnapshot> {
        self.community_mek_impl(community_id)
    }

    fn channel_mek(&self, community_id: &str, channel_id: &str) -> Option<MekSnapshot> {
        self.channel_mek_impl(community_id, channel_id)
    }

    fn channel_meks_all(&self, community_id: &str) -> Vec<ChannelMekSnapshot> {
        self.channel_meks_all_impl(community_id)
    }

    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot) {
        self.insert_community_mek_impl(community_id, &mek);
    }

    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot) {
        self.insert_channel_mek_impl(community_id, channel_id, &mek);
    }

    fn load_historical_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MekSnapshot> {
        self.load_historical_channel_mek_impl(community_id, channel_id, generation)
    }

    // ---------- Bootstrap ----------

    /// The daemon has no message store, so a daemon-served bundle
    /// carries no backlog.
    ///
    /// This is correct rather than a shortfall: the architecture calls
    /// the BootstrapBundle "convenience, not trust" with the DHT
    /// authoritative, and join step 15 is a full SMPL catchup that
    /// rebuilds history from the channel records regardless. A joiner
    /// bootstrapped by a daemon waits longer for backlog; it does not
    /// lose any. Inventing a message store to fill this would add a
    /// second source of truth for content the DHT already holds.
    async fn recent_channel_messages(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _limit: i64,
    ) -> Vec<RecentMessageRow> {
        Vec::new()
    }

    // ---------- Gossip ----------

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), GovernanceRuntimeError> {
        self.send_to_mesh_impl(community_id, envelope);
        Ok(())
    }

    // ---------- Permissions ----------

    fn require_permission(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission_impl(community_id, perm_bits)
    }

    // ---------- Events ----------

    fn emit_event(&self, event: GovernanceRuntimeEvent) {
        self.emit_event_impl(&event);
    }

    // ---------- Background lifecycle ----------

    fn spawn_inspect_loop(&self, community_id: &str) {
        Self::spawn_inspect_loop_impl(community_id);
    }

    fn spawn_presence_poll(&self, community_id: &str) {
        Self::spawn_presence_poll_impl(community_id);
    }

    fn spawn_dht_keepalive(&self, community_id: &str) {
        Self::spawn_dht_keepalive_impl(community_id);
    }

    fn spawn_history_catchup(&self, community_id: &str) {
        Self::spawn_history_catchup_impl(community_id);
    }

    async fn watch_community_records(
        &self,
        community_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.watch_community_records_impl(community_id).await;
        Ok(())
    }

    /// No daemon equivalent yet.
    ///
    /// The Lost Cargo chunk cache is a Tauri-side surface
    /// (`ChunkCache`, filesystem-backed, per-community). The daemon has
    /// no file-transfer subsystem, so there is nothing to open. Recorded
    /// as a feature gap rather than silently satisfied — see the CLI
    /// parity note in the module header.
    fn ensure_files_cache_open(&self, community_id: &str) {
        tracing::debug!(
            community_id,
            "governance adapter: files cache not implemented on the daemon track"
        );
    }

    fn persist_discovered_registry_members(
        &self,
        community_id: &str,
        members: Vec<DiscoveredMember>,
    ) {
        Self::persist_discovered_registry_members_impl(community_id, &members);
    }

    // ---------- Join flow ----------

    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        self.app_call_peer_impl(target_route_blob, payload).await
    }

    fn rebuild_governance_state(
        &self,
        entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    ) -> GovernanceState {
        Self::rebuild_governance_state_impl(&entries)
    }

    // ---------- DHT hydration ----------

    fn list_community_governance_targets(&self) -> Vec<(String, String)> {
        self.list_community_governance_targets_impl()
    }

    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)> {
        self.list_registries_with_my_pseudonym_impl()
    }

    fn apply_recovered_member_state(
        &self,
        community_id: &str,
        subkey_index: u32,
        role_ids: &[u32],
    ) {
        self.apply_recovered_member_state_impl(community_id, subkey_index, role_ids);
    }

    fn try_derive_slot_keypair_if_ready(&self, community_id: &str) {
        // Nothing to do: this track derives the slot keypair on demand
        // in `community_membership` from the seed plus the slot index,
        // rather than caching it in state. There is no "not yet derived"
        // condition to repair.
        let _ = community_id;
    }

    fn list_missing_registry_keypairs(&self) -> Vec<String> {
        // The daemon never persists a registry owner keypair: under
        // `o_cnt: 0` there is no owner writer, and member slots use the
        // derived slot keypair. So none can be missing.
        Vec::new()
    }

    fn recover_registry_keypair_from_keystore(&self, community_id: &str) {
        let _ = community_id;
    }

    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup> {
        self.list_communities_for_dht_open_impl()
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<String> {
        self.channel_log_keys_for_community_impl(community_id)
    }

    fn list_my_active_invite_secret_keys(&self) -> Vec<String> {
        Self::list_my_active_invite_secret_keys_impl()
    }

    fn track_open_dht_records(&self, keys: &[String]) {
        // Keyed per community on this track; the orchestrator's bulk
        // call has no community context, so records are tracked as they
        // are opened in `mark_open_channel_record` /
        // `mark_community_records_open` instead.
        let _ = keys;
    }

    fn register_governance_overflow_keys(&self, community_id: &str, keys: &[String]) {
        self.register_governance_overflow_keys_impl(community_id, keys);
    }

    fn governance_overflow_keys_for_community(&self, community_id: &str) -> Vec<String> {
        self.governance_overflow_keys_impl(community_id)
    }

    fn mark_community_records_open(
        &self,
        community_id: &str,
        governance_key: &str,
        registry_key: Option<&str>,
        _registry_writer: Option<&str>,
        channel_keys: Vec<String>,
    ) {
        let mut keys = vec![governance_key.to_string()];
        keys.extend(registry_key.map(ToString::to_string));
        keys.extend(channel_keys);
        self.track_open_dht_records_impl(community_id, &keys);
    }

    async fn watch_community_records_post_open(&self, community_id: &str) {
        self.watch_community_records_impl(community_id).await;
    }

    async fn apply_governance_rebuild_result(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
        max_lamport: u64,
    ) {
        self.apply_governance_rebuild_result_impl(community_id, gov_state, max_lamport);
    }

    fn persist_governance_entries_cache(
        &self,
        community_id: &str,
        entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    ) {
        self.persist_governance_entries_cache_impl(community_id, entries);
    }

    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str) {
        self.spawn_text_mek_rotation_for_ban_impl(community_id, banned_pseudonym_hex);
    }

    // ---------- Role mutations ----------

    fn role_current_definition(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Option<RoleSnapshotInsert> {
        self.role_current_definition_impl(community_id, role_id)
    }

    fn role_table_summary(&self, community_id: &str) -> (Vec<u32>, i32) {
        self.role_table_summary_impl(community_id)
    }

    async fn apply_role_assignment(
        &self,
        community_id: &str,
        _pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        self.apply_role_assignment_impl(community_id, role_id, is_self);
        Ok(())
    }

    async fn apply_role_unassignment(
        &self,
        community_id: &str,
        _pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        self.apply_role_unassignment_impl(community_id, role_id, is_self);
        Ok(())
    }

    /// No local mirror to update: the definition is already carried by
    /// the governance entry that triggered this call, and
    /// `role_current_definition` reads it back from the merged CRDT
    /// state. See `roles.rs` for why the daemon keeps no role table.
    async fn apply_role_create(
        &self,
        _community_id: &str,
        _snapshot: RoleSnapshotInsert,
    ) -> Result<(), GovernanceRuntimeError> {
        Ok(())
    }

    /// Likewise: the patch is already in the governance entry and
    /// therefore in the next merge.
    async fn apply_role_edit(
        &self,
        _community_id: &str,
        _role_id: u32,
        _patch: RoleSnapshotPatch,
    ) -> Result<(), GovernanceRuntimeError> {
        Ok(())
    }

    async fn apply_role_delete(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Result<(), GovernanceRuntimeError> {
        self.apply_role_delete_impl(community_id, role_id);
        Ok(())
    }
}
