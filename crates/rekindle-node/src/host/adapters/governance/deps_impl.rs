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
    GovernanceOpStore, GovernanceRuntimeDeps, MekSnapshot, OnlineMemberSnapshot, RoleTable,
    SlotRecovery, UserStatusKind,
};
use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_governance_runtime::CardText;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_records::port::RecordSchema;
use rekindle_types::governance::GenesisCert;
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

    fn device_op_key(
        &self,
        community_id: &str,
    ) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, GovernanceRuntimeError> {
        self.device_op_key_impl(community_id)
    }

    fn store_device_op_key(
        &self,
        community_id: &str,
        secret: &[u8; 32],
    ) -> Result<(), GovernanceRuntimeError> {
        self.store_device_op_key_impl(community_id, secret)
    }

    fn store_device_record(
        &self,
        community_id: &str,
        record: &str,
        owner: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.store_device_record_impl(community_id, record, owner)
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

    async fn next_beat_seq(&self, community_id: &str) -> Result<u64, GovernanceRuntimeError> {
        self.next_beat_seq_impl(community_id).await
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
        intent: rekindle_records::port::OpenIntent,
    ) -> Result<LeaseId, GovernanceRuntimeError> {
        self.acquire_record_impl(record_key, writer, intent).await
    }

    async fn release_record(&self, lease: LeaseId) {
        self.release_record_impl(lease).await;
    }

    async fn discard_created_community(&self, community_id: &str) {
        self.discard_created(community_id).await;
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

    async fn read_hinted_value(
        &self,
        lease: LeaseId,
        subkey: u32,
    ) -> Result<Option<(u64, Vec<u8>)>, GovernanceRuntimeError> {
        self.read_hinted_value_impl(lease, subkey).await
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

    fn unlock_barrier(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<rekindle_governance_runtime::unlock_barrier::UnlockBarrier> {
        self.ctx.communities.unlock_barriers().of(community_id)
    }

    fn emit_catch_up(
        &self,
        community_id: &str,
        view: rekindle_types::display_catch_up::CatchUpView,
    ) {
        self.ctx.publish_event(
            rekindle_types::subscription_events::SubscriptionEvent::Governance(
                rekindle_types::subscription_events::GovernanceEvent::CatchUp {
                    community: community_id.to_owned(),
                    catching_up: view.catching_up,
                    pending: view.pending,
                },
            ),
        );
    }

    fn pending_target_name(
        &self,
        community_id: &str,
        target: &rekindle_types::display_catch_up::PendingGovTarget,
    ) -> Option<String> {
        self.pending_target_name_impl(community_id, target)
    }

    fn handle(&self) -> Self {
        self.clone()
    }

    fn spawn_held_write(
        &self,
        community_id: &str,
        write: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    ) {
        self.ctx
            .community_scope(community_id)
            .spawn_or_drop("held governance write", write);
    }

    // ---------- Background lifecycle ----------

    fn spawn_history_catchup(&self, community_id: &str) {
        self.spawn_outbox_catchup(community_id);
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

    fn list_communities_for_dht_open(&self) -> Vec<CommunityDhtOpenSetup> {
        self.list_communities_for_dht_open_impl()
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

    fn rebuild_lock(&self, community_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        self.ctx.communities.rebuild_lock(community_id)
    }

    fn admission_lock(&self, community_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        self.ctx.communities.admission_lock(community_id)
    }

    fn rebuild_flight(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<rekindle_governance_runtime::rebuild_flight::RebuildFlight> {
        self.ctx.communities.rebuild_flight(community_id)
    }

    fn op_verify_cache(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<rekindle_governance_runtime::verify_cache::OpVerifyCache> {
        self.ctx.communities.op_verify_cache(community_id)
    }

    fn successor_judgements(
        &self,
        community_id: &str,
    ) -> std::sync::Arc<rekindle_governance_runtime::successor::SuccessorJudgements> {
        self.ctx.communities.successor_judgements(community_id)
    }

    // ---------- Governance ops ----------

    fn genesis_cert(&self, community_id: &str) -> Option<GenesisCert> {
        self.genesis_cert_impl(community_id)
    }

    /// From the cards `member_card::read_card` verified (owner pinned to
    /// the member's `MemberJoined`, signed by the pseudonym).
    fn member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<String> {
        self.ctx
            .communities
            .member_card_name(community_id, pseudonym)
    }

    fn note_member_card(&self, community_id: &str, pseudonym: &PseudonymKey, card: CardText) {
        self.note_member_card_impl(community_id, pseudonym, card);
    }

    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey> {
        let map = self.ctx.communities.map().read();
        let mine = map.get(community_id)?.my_pseudonym_key.as_deref()?;
        super::ops::pseudonym_of(mine)
    }

    fn record_port(
        &self,
    ) -> Result<std::sync::Arc<dyn rekindle_records::port::RecordPort>, GovernanceRuntimeError>
    {
        self.transport()?
            .require_records()
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("record pool: {e}")))
    }

    fn warm_checkpoint_pages(
        &self,
        community_id: &str,
        checkpoint: &rekindle_types::governance::Checkpoint,
    ) {
        let me = self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .and_then(super::watch_ring::my_pseudonym);
        self.ctx
            .communities
            .warm_checkpoint_pages(community_id, checkpoint, me.as_ref());
    }

    fn gossip_governance_op(&self, community_id: &str, encoded: &[u8]) {
        self.gossip_op(community_id, encoded.to_vec());
    }

    async fn publish_governance_op(
        &self,
        community_id: &str,
        encoded: Vec<u8>,
        landing: rekindle_governance_runtime::OpLanding,
    ) -> Result<rekindle_governance_runtime::OpPublication, GovernanceRuntimeError> {
        self.publish_governance_op_impl(community_id, encoded, landing)
            .await
    }

    fn spawn_text_mek_rotation_for_ban(&self, community_id: &str, banned_pseudonym_hex: &str) {
        match &self.ctx.shell {
            Some(shell) => shell.spawn_departure_rotation(community_id, banned_pseudonym_hex),
            None => self.spawn_text_mek_rotation_for_ban_impl(community_id, banned_pseudonym_hex),
        }
    }

    fn spawn_member_card_refresh(&self, community_id: &str) {
        self.schedule_card_refresh(community_id);
    }

    async fn departed_slot_heads(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Result<Vec<rekindle_types::governance::SlotHead>, GovernanceRuntimeError> {
        self.departed_slot_heads_impl(community_id, target_hex)
            .await
    }

    fn op_store(&self) -> std::sync::Arc<dyn GovernanceOpStore> {
        std::sync::Arc::new(self.clone())
    }

    // ---------- Role mutations ----------

    fn roles(&self) -> std::sync::Arc<dyn RoleTable> {
        std::sync::Arc::new(self.clone())
    }

    fn slot_recovery(&self) -> std::sync::Arc<dyn SlotRecovery> {
        std::sync::Arc::new(self.clone())
    }
}
