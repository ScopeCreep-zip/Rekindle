//! Phase 18.h — `GovernanceRuntimeDeps` trait impl for `GovernanceAdapter`.
//!
//! Trait methods delegate to focused helpers in sibling submodules
//! (`dht`, `events`, `state_mutations`); see `governance_adapter::mod`
//! for the module map.

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_crypto::group::media_key::MediaEncryptionKey as CryptoMek;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::{
    CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo, DiscoveredMember,
    GovernanceRuntimeDeps, GovernanceRuntimeError, GovernanceRuntimeEvent, MekSnapshot,
    OnlineMemberSnapshot, RecentMessageRow, UserStatusKind,
};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::services::community::create::slot_signing_to_veilid;
use crate::state::UserStatus;
use crate::state_helpers;

use super::state_builder::insert_community_into_state;
use super::{dht, events, roles, state_mutations, state_reads, GovernanceAdapter};

#[async_trait]
impl GovernanceRuntimeDeps for GovernanceAdapter {
    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        crate::state_helpers::login_scope_or_closed(&self.state)
    }

    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        state_helpers::identity_secret(&self.state)
    }

    fn identity_display_name(&self) -> String {
        state_helpers::identity_display_name(&self.state)
    }

    fn identity_status(&self) -> UserStatusKind {
        match state_helpers::identity_status(&self.state).unwrap_or_default() {
            UserStatus::Online => UserStatusKind::Online,
            UserStatus::Away => UserStatusKind::Away,
            UserStatus::Busy => UserStatusKind::Busy,
            UserStatus::Offline => UserStatusKind::Offline,
            UserStatus::Invisible => UserStatusKind::Invisible,
        }
    }

    fn our_route_blob(&self) -> Vec<u8> {
        state_helpers::our_route_blob(&self.state).unwrap_or_default()
    }

    // ---------- Community state (read) ----------

    fn community_membership(&self, community_id: &str) -> Option<CommunityMembership> {
        state_reads::community_membership_impl(self, community_id)
    }

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        state_helpers::governance_state(&self.state, community_id)
    }

    fn online_members(&self, community_id: &str) -> Vec<OnlineMemberSnapshot> {
        state_reads::online_members_impl(self, community_id)
    }

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        state_helpers::set_governance_state(&self.state, community_id, state);
    }

    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        state_helpers::next_governance_lamport(&self.state, community_id)
    }

    fn insert_community(&self, community: CommunityInsert) {
        insert_community_into_state(&self.state, community);
    }

    // ---------- DHT ----------

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        dht::create_smpl_record_impl(self, member_pubkeys).await
    }

    async fn create_dflt_record(&self) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        dht::create_dflt_record_impl(self).await
    }

    async fn create_overflow_record(
        &self,
        owner_keypair: String,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        dht::create_overflow_record_impl(self, owner_keypair).await
    }

    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        let sk = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&ed_secret);
        debug_assert_eq!(sk.verifying_key().to_bytes(), ed_public);
        slot_signing_to_veilid(&sk).to_string()
    }

    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<rekindle_records::lease::LeaseId, GovernanceRuntimeError> {
        dht::acquire_record_impl(self, record_key, writer).await
    }

    async fn release_record(&self, lease: rekindle_records::lease::LeaseId) {
        dht::release_record_impl(self, lease).await;
    }

    async fn community_records_ready(
        &self,
        community_id: &str,
        leases: rekindle_records::lease::CommunityLeases,
    ) {
        crate::services::community::leases::records_ready(&self.state, community_id, leases).await;
    }

    async fn get_dht_value(
        &self,
        lease: rekindle_records::lease::LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        dht::get_dht_value_impl(self, lease, subkey, force_refresh).await
    }

    async fn set_dht_value(
        &self,
        lease: rekindle_records::lease::LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        dht::set_dht_value_impl(self, lease, subkey, value, writer).await
    }

    async fn inspect_dht_record_local_seqs(
        &self,
        lease: rekindle_records::lease::LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        dht::inspect_dht_record_local_seqs_impl(self, lease).await
    }

    async fn inspect_dht_record_update_get_seqs(
        &self,
        lease: rekindle_records::lease::LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        dht::inspect_dht_record_update_get_seqs_impl(self, lease).await
    }

    async fn inspect_dht_record_present_subkeys(
        &self,
        lease: rekindle_records::lease::LeaseId,
    ) -> Result<Vec<u32>, GovernanceRuntimeError> {
        dht::inspect_dht_record_present_subkeys_impl(self, lease).await
    }

    // ---------- MEK cache ----------

    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        crate::state_helpers::key_provider(&self.state)
    }

    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot) {
        // Route through the centralized resolver so a hydrated snapshot can't
        // downgrade a newer live key or clobber a canonical same-gen key.
        // (MekSnapshot carries no provenance → untagged; it loses to any
        // tagged key and is keep-cached vs another untagged one, which is the
        // correct behaviour for hydration.)
        if crate::state_helpers::install_mek(
            &self.state,
            community_id,
            rekindle_types::channel_keys::KeyScope::Community,
            CryptoMek::from_bytes(mek.key_bytes, mek.generation),
        ) {
            crate::services::community::media_ready_runtime::on_mek_updated(
                &self.state,
                community_id,
                None,
            );
        }
    }

    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot) {
        let Some(channel) = rekindle_types::id::ChannelId::from_hex(channel_id) else {
            tracing::warn!(community = %community_id, channel_id, "not a channel id — key not installed");
            return;
        };
        if crate::state_helpers::install_mek(
            &self.state,
            community_id,
            rekindle_types::channel_keys::KeyScope::Channel(channel),
            CryptoMek::from_bytes(mek.key_bytes, mek.generation),
        ) {
            crate::services::community::media_ready_runtime::on_mek_updated(
                &self.state,
                community_id,
                Some(channel_id),
            );
        }
    }

    // ---------- Bootstrap (SQL) ----------

    async fn recent_channel_messages(
        &self,
        community_id: &str,
        channel_id: &str,
        limit: i64,
    ) -> Vec<RecentMessageRow> {
        dht::recent_channel_messages_impl(self, community_id, channel_id, limit).await
    }

    // ---------- Gossip ----------

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), GovernanceRuntimeError> {
        crate::services::community::send_to_mesh(&self.state, community_id, envelope)
            .map_err(GovernanceRuntimeError::Adapter)
    }

    // ---------- Permissions ----------

    fn require_permission(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError> {
        crate::commands::community::require_permission(&self.state, community_id, perm_bits)
            .map_err(|_| GovernanceRuntimeError::PermissionDenied)
    }

    // ---------- Events ----------

    fn emit_event(&self, event: GovernanceRuntimeEvent) {
        events::emit_event_impl(self, event);
    }

    // ---------- Background lifecycle ----------

    fn spawn_history_catchup(&self, community_id: &str) {
        crate::services::community::join::schedule_history_catchup(
            self.state.clone(),
            community_id.to_string(),
        );
    }

    fn ensure_files_cache_open(&self, community_id: &str) {
        if let Err(e) =
            crate::services::community::files::ensure_cache_open(&self.state, community_id)
        {
            tracing::warn!(community = %community_id, error = %e, "Lost Cargo cache unavailable");
        }
    }

    fn persist_discovered_registry_members(
        &self,
        community_id: &str,
        members: Vec<DiscoveredMember>,
    ) {
        state_mutations::persist_discovered_registry_members_impl(self, community_id, members);
    }

    // ---------- Join-flow specific ----------

    async fn app_call_peer(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        dht::app_call_peer_impl(self, target_route_blob, payload).await
    }

    fn rebuild_governance_state(
        &self,
        entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    ) -> GovernanceState {
        rekindle_governance::merge::merge(&entries)
    }

    // ---------- DHT-hydration deps (Phase 23.C chiral split) ----------

    fn list_community_governance_targets(&self) -> Vec<(String, String)> {
        state_helpers::communities_with_governance_keys(&self.state)
    }

    async fn apply_governance_rebuild_result(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
        max_lamport: u64,
    ) {
        state_mutations::apply_governance_rebuild_result_impl(
            self,
            community_id,
            gov_state,
            max_lamport,
        )
        .await;
    }

    fn persist_governance_entries_cache(
        &self,
        community_id: &str,
        entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    ) {
        state_mutations::persist_governance_entries_cache_impl(self, community_id, entries);
    }

    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)> {
        let communities = self.state.communities.read();
        communities
            .iter()
            .filter_map(|(cid, cs)| {
                let rk = cs.member_registry_key.clone()?;
                Some((cid.clone(), rk, cs.my_pseudonym_key.clone()))
            })
            .collect()
    }

    fn apply_recovered_member_state(
        &self,
        community_id: &str,
        subkey_index: u32,
        role_ids: &[u32],
    ) {
        state_mutations::apply_recovered_member_state_impl(
            self,
            community_id,
            subkey_index,
            role_ids,
        );
    }

    fn try_derive_slot_keypair_if_ready(&self, community_id: &str) {
        state_mutations::try_derive_slot_keypair_if_ready_impl(self, community_id);
    }

    fn list_missing_registry_keypairs(&self) -> Vec<String> {
        let communities = self.state.communities.read();
        communities
            .iter()
            .filter(|(_, cs)| {
                cs.member_registry_key.is_some() && cs.registry_owner_keypair.is_none()
            })
            .map(|(cid, _)| cid.clone())
            .collect()
    }

    fn recover_registry_keypair_from_keystore(&self, community_id: &str) {
        state_mutations::recover_registry_keypair_from_keystore_impl(self, community_id);
    }

    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup> {
        let cs = self.state.communities.read();
        cs.values()
            .filter_map(|c| {
                c.governance_key.as_ref().map(|gk| CommunityDhtOpenSetup {
                    id: c.id.clone(),
                    governance_key: gk.clone(),
                    registry_key: c.member_registry_key.clone(),
                    registry_writer: c
                        .registry_owner_keypair
                        .clone()
                        .or_else(|| c.slot_keypair.clone()),
                    slot_writer: c.slot_keypair.clone(),
                })
            })
            .collect()
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<(String, String)> {
        let cs = self.state.communities.read();
        cs.get(community_id)
            .map(|c| {
                c.channel_log_keys
                    .iter()
                    .map(|(id, key)| (id.clone(), key.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn list_my_active_invite_secret_keys(&self) -> Vec<String> {
        state_reads::list_my_active_invite_secret_keys_impl(self)
    }

    fn register_governance_overflow_keys(&self, community_id: &str, keys: &[String]) {
        state_mutations::register_governance_overflow_keys_impl(self, community_id, keys);
    }

    fn governance_overflow_keys_for_community(&self, community_id: &str) -> Vec<String> {
        let cs = self.state.communities.read();
        cs.get(community_id)
            .map(|c| c.open_community_records.governance_overflow_keys.clone())
            .unwrap_or_default()
    }

    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str) {
        state_mutations::spawn_text_mek_rotation_for_ban_impl(
            self,
            community_id,
            banned_pseudonym_hex,
        );
    }

    fn role_current_definition(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Option<rekindle_governance_runtime::roles::RoleSnapshotInsert> {
        roles::role_current_definition_impl(self, community_id, role_id)
    }

    fn role_table_summary(&self, community_id: &str) -> (Vec<u32>, i32) {
        roles::role_table_summary_impl(self, community_id)
    }

    async fn apply_role_assignment(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        roles::apply_role_assignment_impl(self, community_id, pseudonym_key, role_id, is_self).await
    }

    async fn apply_role_unassignment(
        &self,
        community_id: &str,
        pseudonym_key: &str,
        role_id: u32,
        is_self: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        roles::apply_role_unassignment_impl(self, community_id, pseudonym_key, role_id, is_self)
            .await
    }

    async fn apply_role_create(
        &self,
        community_id: &str,
        snapshot: rekindle_governance_runtime::roles::RoleSnapshotInsert,
    ) -> Result<(), GovernanceRuntimeError> {
        roles::apply_role_create_impl(self, community_id, snapshot).await
    }

    async fn apply_role_edit(
        &self,
        community_id: &str,
        role_id: u32,
        patch: rekindle_governance_runtime::roles::RoleSnapshotPatch,
    ) -> Result<(), GovernanceRuntimeError> {
        roles::apply_role_edit_impl(self, community_id, role_id, patch).await
    }

    async fn apply_role_delete(
        &self,
        community_id: &str,
        role_id: u32,
    ) -> Result<(), GovernanceRuntimeError> {
        roles::apply_role_delete_impl(self, community_id, role_id).await
    }
}
