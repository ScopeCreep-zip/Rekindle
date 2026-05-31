//! Phase 19.h-REDO — `ChannelMessagingDeps` trait impl for `ChannelAdapter`.
//!
//! Trait methods delegate to focused helpers in sibling submodules
//! (`state_reads`, `dht`, `persist`, `events`); see
//! `channel_adapter::mod` for the module map.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_channel::deps::{
    ChannelEntryItem, ChannelInfoSnapshot, ChannelMek, ChannelMessageRow, ChannelMessagingDeps,
    ChannelSendOutcome, ChannelWriteContext, MemberProfileSnapshot, PendingChannelWrite,
    PseudonymCredentials, RoleSnapshot, SentChannelMessageEcho, ThreadInfoSnapshot,
    ThreadStateSnapshot,
};
use rekindle_channel::error::ChannelError;
use rekindle_channel::event::ChannelEvent;
use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::channel_record::{
    ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose, ChannelPollCreate,
    ChannelPollVote, ChannelReaction,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::GovernanceEntry;

use crate::state_helpers;

use super::{
    dht, events, misc, persist, state_mutations, state_reads, ChannelAdapter,
};

#[async_trait]
impl ChannelMessagingDeps for ChannelAdapter {
    // ---------- Identity / credentials ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.state.identity_secret.lock().as_ref().copied()
    }

    fn owner_key(&self) -> Option<String> {
        state_helpers::current_owner_key(&self.state).ok()
    }

    fn my_pseudonym_hex(&self, community_id: &str) -> Option<String> {
        self.state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    }

    fn my_role_ids(&self, community_id: &str) -> Vec<u32> {
        self.state
            .communities
            .read()
            .get(community_id)
            .map(|c| c.my_role_ids.clone())
            .unwrap_or_default()
    }

    fn pseudonym_credentials(
        &self,
        community_id: &str,
    ) -> Result<PseudonymCredentials, ChannelError> {
        let (pseudonym, signing_key) =
            state_helpers::pseudonym_credentials(&self.state, community_id)
                .map_err(ChannelError::Adapter)?;
        Ok(PseudonymCredentials {
            pseudonym,
            signing_key,
        })
    }

    // ---------- Community / channel state (read) ----------

    fn channel_info(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<ChannelInfoSnapshot> {
        state_reads::channel_info_impl(self, community_id, channel_id)
    }

    fn channel_write_context(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<ChannelWriteContext, ChannelError> {
        state_reads::channel_write_context_impl(self, community_id, channel_id)
    }

    fn channel_record_key(&self, community_id: &str, channel_id: &str) -> Option<String> {
        self.state
            .communities
            .read()
            .get(community_id)?
            .channel_log_keys
            .get(channel_id)
            .cloned()
    }

    fn community_mek(&self, community_id: &str) -> Option<ChannelMek> {
        state_mutations::community_mek_impl(self, community_id)
    }

    fn channel_or_community_mek(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<ChannelMek> {
        state_mutations::channel_or_community_mek_impl(self, community_id, channel_id)
    }

    fn current_mek_generation(&self, community_id: &str) -> Option<u64> {
        state_mutations::current_mek_generation_impl(self, community_id)
    }

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        state_helpers::governance_state(&self.state, community_id)
    }

    fn automod_compiled_cache_get(
        &self,
        community_id: &str,
    ) -> Option<std::sync::Arc<rekindle_channel::AutoModCompiledCache>> {
        self.state.automod_cache.read().get(community_id).cloned()
    }

    fn automod_compiled_cache_set(
        &self,
        community_id: &str,
        cache: std::sync::Arc<rekindle_channel::AutoModCompiledCache>,
    ) {
        self.state
            .automod_cache
            .write()
            .insert(community_id.to_string(), cache);
    }

    fn thread_state(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadStateSnapshot> {
        state_reads::thread_state_impl(self, community_id, thread_id)
    }

    fn slot_seed_bytes(&self, community_id: &str) -> Option<[u8; 32]> {
        let seed_hex = self
            .state
            .communities
            .read()
            .get(community_id)?
            .slot_seed
            .clone()?;
        hex::decode(seed_hex).ok().and_then(|b| b.try_into().ok())
    }

    fn member_profile(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
    ) -> MemberProfileSnapshot {
        state_reads::member_profile_impl(self, community_id, pseudonym_hex)
    }

    fn list_member_profiles(
        &self,
        community_id: &str,
    ) -> HashMap<String, MemberProfileSnapshot> {
        state_reads::list_member_profiles_impl(self, community_id)
    }

    fn community_roles(&self, community_id: &str) -> Vec<RoleSnapshot> {
        state_reads::community_roles_impl(self, community_id)
    }

    fn compute_my_permissions(&self, community_id: &str) -> u64 {
        state_reads::compute_my_permissions_impl(self, community_id)
    }

    // ---------- Community / channel state (mutation) ----------

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64 {
        state_mutations::next_channel_sequence_impl(self, community_id, channel_id)
    }

    fn next_thread_sequence(&self, community_id: &str) -> u64 {
        state_mutations::next_thread_sequence_impl(self, community_id)
    }

    fn mark_last_send_at(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        state_mutations::mark_last_send_at_impl(self, community_id, channel_id, now_ms);
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn track_open_records(&self, _community_id: &str, record_keys: &[String]) {
        state_helpers::track_open_records(&self.state, record_keys);
    }

    // ---------- DHT ----------

    async fn write_channel_message_smpl(
        &self,
        context: &ChannelWriteContext,
        channel_msg: &ChannelMessage,
    ) -> Result<(), ChannelError> {
        dht::write_channel_message_smpl_impl(self, context, channel_msg).await
    }

    async fn write_channel_forward_smpl(
        &self,
        context: &ChannelWriteContext,
        forward: &ChannelForward,
    ) -> Result<(), ChannelError> {
        dht::write_channel_forward_smpl_impl(self, context, forward).await
    }

    async fn write_member_reaction_smpl(
        &self,
        context: &ChannelWriteContext,
        reaction: &ChannelReaction,
    ) -> Result<(), ChannelError> {
        dht::write_member_reaction_smpl_impl(self, context, reaction).await
    }

    async fn write_channel_poll_create_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollCreate,
    ) -> Result<(), ChannelError> {
        dht::write_channel_poll_create_smpl_impl(self, context, entry).await
    }

    async fn write_channel_poll_vote_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollVote,
    ) -> Result<(), ChannelError> {
        dht::write_channel_poll_vote_smpl_impl(self, context, entry).await
    }

    async fn write_channel_poll_close_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollClose,
    ) -> Result<(), ChannelError> {
        dht::write_channel_poll_close_smpl_impl(self, context, entry).await
    }

    async fn write_channel_hand_raise_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelHandRaise,
    ) -> Result<(), ChannelError> {
        dht::write_channel_hand_raise_smpl_impl(self, context, entry).await
    }

    async fn stage_pseudonyms_by_subkey(
        &self,
        community_id: &str,
    ) -> Result<std::collections::HashMap<u32, String>, ChannelError> {
        persist::stage_pseudonyms_by_subkey_impl(self, community_id).await
    }

    async fn create_smpl_thread_record(
        &self,
        slot_seed_bytes: &[u8; 32],
    ) -> Result<String, ChannelError> {
        dht::create_smpl_thread_record_impl(self, slot_seed_bytes).await
    }

    async fn read_all_channel_entries(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelEntryItem>, ChannelError> {
        dht::read_all_channel_entries_impl(self, record_key, member_count).await
    }

    async fn read_all_channel_messages(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelMessage>, ChannelError> {
        dht::read_all_channel_messages_impl(self, record_key, member_count).await
    }

    async fn watch_community_records(
        &self,
        community_id: &str,
    ) -> Result<(), ChannelError> {
        crate::services::community::watch::watch_community_records(&self.state, community_id)
            .await
            .map_err(ChannelError::Adapter)
    }

    async fn ensure_channel_segment_record(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<String, ChannelError> {
        crate::services::community::segments::ensure_channel_segment_record(
            &self.state,
            community_id,
            channel_id,
        )
        .await
        .map_err(ChannelError::Adapter)
    }

    // ---------- Retry queue ----------

    async fn enqueue_channel_retry(
        &self,
        pending: PendingChannelWrite,
    ) -> Result<(), ChannelError> {
        persist::enqueue_channel_retry_impl(self, pending).await
    }

    // ---------- DB (channel messages) ----------

    async fn persist_sent_message(
        &self,
        _community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
    ) -> Result<(), ChannelError> {
        persist::persist_sent_message_impl(self, channel_id, outcome, body).await
    }

    async fn persist_forwarded_message(
        &self,
        _community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
        original_author_pseudonym: &str,
    ) -> Result<(), ChannelError> {
        persist::persist_forwarded_message_impl(
            self,
            channel_id,
            outcome,
            body,
            original_author_pseudonym,
        )
        .await
    }

    async fn persist_channel_sequence(
        &self,
        community_id: &str,
        channel_id: &str,
        sequence: u64,
    ) -> Result<(), ChannelError> {
        persist::persist_channel_sequence_impl(self, community_id, channel_id, sequence)
    }

    async fn persist_slowmode_state(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), ChannelError> {
        persist::persist_slowmode_state_impl(self, community_id, channel_id, now_ms)
    }

    async fn find_channel_message_by_id(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<ChannelMessageRow> {
        persist::find_channel_message_by_id_impl(self, channel_id, message_id).await
    }

    // ---------- DB (threads) ----------

    async fn persist_thread_row(
        &self,
        community_id: &str,
        thread: &ThreadInfoSnapshot,
    ) -> Result<(), ChannelError> {
        persist::persist_thread_row_impl(self, community_id, thread).await
    }

    async fn load_thread_metadata(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadInfoSnapshot> {
        persist::load_thread_metadata_impl(self, community_id, thread_id).await
    }

    // ---------- Expressions / Mesh / Governance / Permissions ----------

    fn upload_expression_to_cache(
        &self,
        community_id: &str,
        expression_id: [u8; 16],
        bytes: &[u8],
        filename: String,
        mime_type: String,
    ) -> Result<AttachmentOffer, ChannelError> {
        misc::upload_expression_to_cache_impl(
            self,
            community_id,
            expression_id,
            bytes,
            filename,
            mime_type,
        )
    }

    fn read_expression_bytes(
        &self,
        community_id: &str,
        offer: &AttachmentOffer,
    ) -> Option<Vec<u8>> {
        misc::read_expression_bytes_impl(self, community_id, offer)
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), ChannelError> {
        misc::send_to_mesh_impl(self, community_id, envelope)
    }

    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: GovernanceEntry,
    ) -> Result<(), ChannelError> {
        misc::write_governance_entry_impl(self, community_id, entry).await
    }

    fn require_channel_permission(
        &self,
        community_id: &str,
        _channel_id: Option<&str>,
        perm_bits: u64,
    ) -> Result<(), ChannelError> {
        misc::require_channel_permission_impl(self, community_id, perm_bits)
    }

    // ---------- Events ----------

    fn emit_event(&self, event: ChannelEvent) {
        events::emit_event_impl(self, event);
    }

    fn emit_chat_event_local(&self, echo: &SentChannelMessageEcho) {
        events::emit_chat_event_local_impl(self, echo);
    }

    fn emit_delivery_succeeded(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    ) {
        events::emit_delivery_succeeded_impl(self, community_id, channel_id, message_id);
    }

    fn emit_delivery_failed(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    ) {
        events::emit_delivery_failed_impl(self, community_id, channel_id, message_id);
    }
}
