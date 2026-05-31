//! Phase 18.h — `GovernanceRuntimeDeps` trait impl for `GovernanceAdapter`.
//!
//! Trait methods delegate to focused helpers in sibling submodules
//! (`dht`, `events`, `state_mutations`); see `governance_adapter::mod`
//! for the module map.

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey as CryptoMek;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::{
    ChannelMekSnapshot, CommunityDhtOpenSetup, CommunityInsert, CommunityMembership, DhtRecordInfo,
    DiscoveredMember, GovernanceRuntimeDeps, GovernanceRuntimeError, GovernanceRuntimeEvent,
    MekSnapshot, MemberIndexRow, OnlineMemberSnapshot, RecentMessageRow, UserStatusKind,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::services::community::create::slot_signing_to_veilid;
use crate::state::UserStatus;
use crate::state_helpers;

use super::state_builder::insert_community_into_state;
use super::{dht, events, roles, state_mutations, state_reads, GovernanceAdapter};

#[async_trait]
impl GovernanceRuntimeDeps for GovernanceAdapter {
    // ---------- Identity ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.state.identity_secret.lock().as_ref().copied()
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

    fn open_record_keys(&self, community_id: &str) -> Vec<String> {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .map(|cs| {
                cs.open_community_records
                    .channel_keys
                    .iter()
                    .cloned()
                    .chain(cs.open_community_records.registry_key.iter().cloned())
                    .chain(cs.open_community_records.governance_key.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---------- Community state (mutation) ----------

    fn set_governance_state(&self, community_id: &str, state: GovernanceState) {
        state_helpers::set_governance_state(&self.state, community_id, state);
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn insert_community(&self, community: CommunityInsert) {
        insert_community_into_state(&self.state, community);
    }

    fn mark_open_channel_record(&self, community_id: &str, record_key: String) {
        let mut communities = self.state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            if !cs.open_community_records.channel_keys.contains(&record_key) {
                cs.open_community_records.channel_keys.push(record_key);
            }
        }
    }

    // ---------- DHT ----------

    async fn create_smpl_record(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        dht::create_smpl_record_impl(self, member_pubkeys).await
    }

    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        let sk = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&ed_secret);
        debug_assert_eq!(sk.verifying_key().to_bytes(), ed_public);
        slot_signing_to_veilid(&sk).to_string()
    }

    async fn get_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        dht::get_dht_value_impl(self, record_key, subkey, force_refresh).await
    }

    async fn set_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        dht::set_dht_value_impl(self, record_key, subkey, value, writer).await
    }

    async fn inspect_dht_record_local_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        dht::inspect_dht_record_local_seqs_impl(self, record_key).await
    }

    async fn inspect_dht_record_update_get_seqs(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        dht::inspect_dht_record_update_get_seqs_impl(self, record_key).await
    }

    async fn open_dht_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError> {
        dht::open_dht_record_impl(self, record_key, writer).await
    }

    // ---------- MEK cache ----------

    fn community_mek(&self, community_id: &str) -> Option<MekSnapshot> {
        self.state
            .mek_cache
            .lock()
            .get(community_id)
            .map(|mek| MekSnapshot {
                generation: mek.generation(),
                key_bytes: *mek.as_bytes(),
            })
    }

    fn channel_mek(&self, community_id: &str, channel_id: &str) -> Option<MekSnapshot> {
        self.state
            .channel_mek_cache
            .lock()
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map(|mek| MekSnapshot {
                generation: mek.generation(),
                key_bytes: *mek.as_bytes(),
            })
    }

    fn channel_meks_all(&self, community_id: &str) -> Vec<ChannelMekSnapshot> {
        self.state
            .channel_mek_cache
            .lock()
            .iter()
            .filter(|((cid, _), _)| cid == community_id)
            .map(|((_, ch), mek)| ChannelMekSnapshot {
                channel_id: ch.clone(),
                mek: MekSnapshot {
                    generation: mek.generation(),
                    key_bytes: *mek.as_bytes(),
                },
            })
            .collect()
    }

    fn insert_community_mek(&self, community_id: &str, mek: MekSnapshot) {
        self.state.mek_cache.lock().insert(
            community_id.to_string(),
            CryptoMek::from_bytes(mek.key_bytes, mek.generation),
        );
    }

    fn insert_channel_mek(&self, community_id: &str, channel_id: &str, mek: MekSnapshot) {
        self.state.channel_mek_cache.lock().insert(
            (community_id.to_string(), channel_id.to_string()),
            CryptoMek::from_bytes(mek.key_bytes, mek.generation),
        );
    }

    fn load_historical_channel_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MekSnapshot> {
        state_reads::load_historical_channel_mek_impl(self, community_id, channel_id, generation)
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
        let perms = Permissions::from_bits_truncate(perm_bits);
        crate::commands::community::require_permission(&self.state, community_id, perms)
            .map_err(|_| GovernanceRuntimeError::PermissionDenied)
    }

    // ---------- Events ----------

    fn emit_event(&self, event: GovernanceRuntimeEvent) {
        events::emit_event_impl(self, event);
    }

    // ---------- Background lifecycle ----------

    fn spawn_inspect_loop(&self, community_id: &str) {
        crate::services::community::inspect::start_inspect_loop(
            self.state.clone(),
            community_id.to_string(),
        );
    }

    fn spawn_presence_poll(&self, community_id: &str) {
        crate::services::community::presence::start_presence_poll(
            &self.state,
            community_id.to_string(),
        );
    }

    fn spawn_dht_keepalive(&self, community_id: &str) {
        crate::services::community::keepalive::start_dht_keepalive(
            self.state.clone(),
            community_id.to_string(),
        );
    }

    fn spawn_history_catchup(&self, community_id: &str) {
        crate::services::community::join::schedule_history_catchup(
            self.state.clone(),
            community_id.to_string(),
        );
    }

    async fn watch_community_records(
        &self,
        community_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        crate::services::community::watch::watch_community_records(&self.state, community_id)
            .await
            .map_err(GovernanceRuntimeError::Adapter)
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
        let cs = self.state.communities.read();
        cs.values()
            .filter_map(|c| {
                c.governance_key
                    .as_ref()
                    .map(|gk| (c.id.clone(), gk.clone()))
            })
            .collect()
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

    async fn read_member_index_for_registry(
        &self,
        registry_key: &str,
    ) -> Result<Vec<MemberIndexRow>, GovernanceRuntimeError> {
        dht::read_member_index_for_registry_impl(self, registry_key).await
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
                })
            })
            .collect()
    }

    fn channel_log_keys_for_community(&self, community_id: &str) -> Vec<String> {
        let cs = self.state.communities.read();
        cs.get(community_id)
            .map(|c| c.channel_log_keys.values().cloned().collect())
            .unwrap_or_default()
    }

    fn track_open_dht_records(&self, keys: &[String]) {
        state_helpers::track_open_records(&self.state, keys);
    }

    fn mark_community_records_open(
        &self,
        community_id: &str,
        governance_key: &str,
        registry_key: Option<&str>,
        registry_writer: Option<&str>,
        channel_keys: Vec<String>,
    ) {
        state_mutations::mark_community_records_open_impl(
            self,
            community_id,
            governance_key,
            registry_key,
            registry_writer,
            channel_keys,
        );
    }

    async fn watch_community_records_post_open(&self, community_id: &str) {
        if let Err(error) =
            crate::services::community::watch_community_records(&self.state, community_id).await
        {
            tracing::debug!(
                community = %community_id,
                %error,
                "failed to watch community records after login open",
            );
        }
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
