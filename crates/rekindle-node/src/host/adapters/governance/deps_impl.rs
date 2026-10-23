//! The single `GovernanceRuntimeDeps` impl for the daemon.
//!
//! Delegation only — every body is one call into a sibling module. That
//! is the documented adapter shape (`services-pattern.md` §2) and it
//! keeps this file readable as a map of the trait surface rather than a
//! place where logic hides.

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::{
    CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo, DiscoveredMember,
    GovernanceRuntimeDeps, MekSnapshot, OnlineMemberSnapshot, RecentMessageRow, UserStatusKind,
};
use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_governance_runtime::op_log::{HeldOp, OpFate};
use rekindle_governance_runtime::roles::{RoleSnapshotInsert, RoleSnapshotPatch};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_records::port::RecordSchema;
use rekindle_types::governance::{GenesisCert, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

#[async_trait]
impl GovernanceRuntimeDeps for HostGovernance {
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        self.ctx.unlock_scope_or_closed()
    }

    fn clock(&self) -> std::sync::Arc<dyn rekindle_utils::Clock> {
        std::sync::Arc::clone(&self.ctx.clock)
    }

    fn rng(&self) -> std::sync::Arc<dyn rekindle_utils::RandomSource> {
        std::sync::Arc::clone(&self.ctx.rng)
    }

    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.identity_secret_impl()
    }

    fn identity_display_name(&self) -> String {
        self.identity_display_name_impl()
    }

    fn identity_status(&self) -> UserStatusKind {
        self.identity_status_impl()
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

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        self.set_governance_state_impl(community_id, state);
    }

    fn insert_community(&self, community: CommunityInsert) {
        self.insert_community_impl(community);
    }

    // ---------- DHT ----------

    async fn create_record(
        &self,
        schema: &RecordSchema,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        self.create_record_impl(schema).await
    }

    fn owner_of(&self, lease: LeaseId) -> Result<String, GovernanceRuntimeError> {
        self.owner_of_impl(lease)
    }

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
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        self.create_overflow_record_impl(owner_keypair).await
    }

    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        Self::format_writer_keypair_impl(ed_public, ed_secret)
    }

    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<LeaseId, GovernanceRuntimeError> {
        self.acquire_record_impl(record_key, writer).await
    }

    async fn release_record(&self, lease: LeaseId) {
        self.release_record_impl(lease).await;
    }

    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases) {
        if let Some(shell) = &self.ctx.shell {
            shell.community_records_ready(community_id, leases).await;
            return;
        }
        self.community_records_ready_impl(community_id, leases)
            .await;
    }

    async fn get_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.get_dht_value_impl(lease, subkey, force_refresh).await
    }

    async fn set_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.set_dht_value_impl(lease, subkey, value, writer).await
    }

    async fn inspect_dht_record_local_seqs(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        self.inspect_local_seqs_impl(lease).await
    }

    async fn inspect_dht_record_update_get_seqs(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        self.inspect_network_seqs_impl(lease).await
    }

    async fn inspect_dht_record_present_subkeys(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<u32>, GovernanceRuntimeError> {
        self.inspect_present_subkeys_impl(lease).await
    }

    // ---------- MEK cache ----------

    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        match &self.ctx.shell {
            Some(shell) => shell.channel_keys(),
            None => crate::daemon::mek_rotation::key_provider(&self.ctx),
        }
    }

    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot) {
        self.insert_community_mek_impl(community_id, &mek);
    }

    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot) {
        self.insert_channel_mek_impl(community_id, channel_id, &mek);
    }

    // ---------- Bootstrap ----------

    async fn recent_channel_messages(
        &self,
        community_id: &str,
        channel_id: &str,
        limit: i64,
    ) -> Vec<RecentMessageRow> {
        self.recent_channel_messages_impl(community_id, channel_id, limit)
            .await
    }

    // ---------- Gossip ----------

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), GovernanceRuntimeError> {
        if let Some(shell) = &self.ctx.shell {
            return shell
                .send_to_mesh(community_id, envelope)
                .map_err(GovernanceRuntimeError::Adapter);
        }
        self.send_to_mesh_impl(community_id, envelope);
        Ok(())
    }

    // ---------- Permissions ----------

    fn require_permission(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError> {
        if let Some(shell) = &self.ctx.shell {
            return shell
                .require_permission(community_id, perm_bits)
                .map_err(|_| GovernanceRuntimeError::PermissionDenied);
        }
        self.require_permission_impl(community_id, perm_bits)
    }

    // ---------- Events ----------

    fn emit_event(&self, event: GovernanceRuntimeEvent) {
        self.emit_event_impl(&event);
    }

    // ---------- Background lifecycle ----------

    fn spawn_history_catchup(&self, community_id: &str) {
        self.spawn_history_catchup_impl(community_id);
    }

    /// Open the community's chunk cache in the Host's store (plan E3.1 E3c).
    fn ensure_files_cache_open(&self, community_id: &str) {
        if let Err(e) = self.ctx.files.ensure_open(community_id) {
            tracing::warn!(community_id, error = %e, "file cache unavailable");
        }
    }

    fn persist_discovered_registry_members(
        &self,
        community_id: &str,
        members: Vec<DiscoveredMember>,
    ) {
        self.persist_discovered_registry_members_impl(community_id, &members);
    }

    // ---------- Join flow ----------

    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        self.app_call_peer_impl(target_route_blob, payload).await
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
        self.try_derive_slot_keypair_impl(community_id);
    }

    fn list_missing_registry_keypairs(&self) -> Vec<String> {
        self.ctx.communities.missing_registry_keypairs()
    }

    fn recover_registry_keypair_from_keystore(&self, community_id: &str) {
        self.recover_registry_keypair_impl(community_id);
    }

    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup> {
        self.list_communities_for_dht_open_impl()
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)> {
        self.channel_log_keys_for_community_impl(community_id)
    }

    fn list_my_active_invite_secret_keys(&self) -> Vec<String> {
        self.list_my_active_invite_secret_keys_impl()
    }

    fn register_governance_overflow_keys(&self, community_id: &str, keys: &[String]) {
        self.register_governance_overflow_keys_impl(community_id, keys);
    }

    fn governance_overflow_keys_for_community(&self, community_id: &str) -> Vec<String> {
        self.governance_overflow_keys_impl(community_id)
    }

    async fn apply_governance_rebuild_result(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
    ) {
        self.apply_governance_rebuild_result_impl(community_id, gov_state)
            .await;
    }

    // ---------- Governance ops ----------

    fn genesis_cert(&self, community_id: &str) -> Option<GenesisCert> {
        self.genesis_cert_impl(community_id)
    }

    /// We hold no verified member card yet: `M`/0 is read once the watch
    /// ring and record warming read member records (plan E3.2 S7).
    fn member_card_name(&self, _community_id: &str, _pseudonym: &PseudonymKey) -> Option<String> {
        None
    }

    async fn governance_ops(
        &self,
        community_id: &str,
    ) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
        self.governance_ops_impl(community_id).await
    }

    async fn store_governance_op(
        &self,
        community_id: &str,
        op: &HeldOp,
    ) -> Result<(), GovernanceRuntimeError> {
        self.store_governance_op_impl(community_id, op).await
    }

    async fn set_governance_op_fates(
        &self,
        community_id: &str,
        fates: Vec<(OpHash, OpFate)>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.set_governance_op_fates_impl(community_id, fates).await
    }

    async fn fetch_governance_op(
        &self,
        op_ref: &OpRef,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.fetch_governance_op_impl(op_ref).await
    }

    fn publish_governance_op(
        &self,
        community_id: &str,
        encoded: Vec<u8>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.publish_governance_op_impl(community_id, encoded)
    }

    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str) {
        match &self.ctx.shell {
            Some(shell) => shell.spawn_departure_rotation(community_id, banned_pseudonym_hex),
            None => self.spawn_text_mek_rotation_for_ban_impl(community_id, banned_pseudonym_hex),
        }
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
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        self.apply_role_assignment_impl(community_id, pseudonym_key, role_id, is_self)
            .await
    }

    async fn apply_role_unassignment(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        self.apply_role_unassignment_impl(community_id, pseudonym_key, role_id, is_self)
            .await
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
        self.apply_role_delete_impl(community_id, role_id).await
    }
}
