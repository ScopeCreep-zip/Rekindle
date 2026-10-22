//! The single `ChannelMessagingDeps` impl for the Host. Delegation only:
//! every body is a call into a sibling module, the governance adapter or
//! the shared crates.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_channel::deps::{
    ChannelEntryItem, ChannelInfoSnapshot, ChannelMessageRow, ChannelMessagingDeps, ChannelPost,
    ChannelSendOutcome, ChannelWriteContext, DhtWrite, MemberMentionView, PseudonymCredentials,
    RoleSnapshot, SentChannelMessageEcho, ThreadInfoSnapshot, ThreadStateSnapshot,
};
use rekindle_channel::error::ChannelError;
use rekindle_channel::event::ChannelEvent;
use rekindle_codec::community::channel_record::{
    ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose, ChannelPollCreate,
    ChannelPollVote, ChannelReaction,
};
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_protocol::dht::community::channel_record as record;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::GovernanceEntry;

use super::dht::delivery;
use super::HostChannel;

/// The thread counter's key among the per-channel sequences.
const THREAD_SEQUENCE: &str = "__thread__";

/// Thread rows and expression blobs are E3.1 E3's (rich messaging, the
/// files domain); the GUI's thread and expression paths run on its own
/// adapter until then, so the Host is never asked for them.
fn not_ported(what: &str) -> ChannelError {
    ChannelError::Adapter(format!("{what} move to the Host at E3.1 E3"))
}

fn adapter(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(e.to_string())
}

#[async_trait]
impl ChannelMessagingDeps for HostChannel {
    // ---------- Identity / credentials ----------

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.ctx().identity_secret()
    }

    fn owner_key(&self) -> Option<String> {
        self.owner_and_db().ok().map(|(owner, _)| owner)
    }

    fn my_pseudonym_hex(&self, community_id: &str) -> Option<String> {
        self.my_pseudonym_impl(community_id)
    }

    fn my_role_ids(&self, community_id: &str) -> Vec<u32> {
        self.my_role_ids_impl(community_id)
    }

    fn pseudonym_credentials(
        &self,
        community_id: &str,
    ) -> Result<PseudonymCredentials, ChannelError> {
        self.pseudonym_credentials_impl(community_id)
    }

    // ---------- Community / channel state (read) ----------

    fn channel_info(&self, community_id: &str, channel_id: &str) -> Option<ChannelInfoSnapshot> {
        self.channel_info_impl(community_id, channel_id)
    }

    fn channel_write_context(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<ChannelWriteContext, ChannelError> {
        self.channel_write_context_impl(community_id, channel_id)
    }

    fn channel_record_key(&self, community_id: &str, channel_id: &str) -> Option<String> {
        self.channel_record_key_impl(community_id, channel_id)
    }

    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        self.gov.keys()
    }

    fn governance_state(&self, community_id: &str) -> Option<GovernanceState> {
        self.gov.governance_state(community_id)
    }

    fn automod_compiled_cache_get(
        &self,
        community_id: &str,
    ) -> Option<std::sync::Arc<rekindle_channel::AutoModCompiledCache>> {
        self.ctx().automod_rules.read().get(community_id).cloned()
    }

    fn automod_compiled_cache_set(
        &self,
        community_id: &str,
        cache: std::sync::Arc<rekindle_channel::AutoModCompiledCache>,
    ) {
        (self.ctx().automod_rules.write()).insert(community_id.to_string(), cache);
    }

    fn thread_state(&self, community_id: &str, thread_id: &str) -> Option<ThreadStateSnapshot> {
        self.thread_state_impl(community_id, thread_id)
    }

    fn slot_seed_bytes(&self, community_id: &str) -> Option<[u8; 32]> {
        self.slot_seed_impl(community_id)
    }

    fn member_profile(&self, community_id: &str, pseudonym_hex: &str) -> MemberMentionView {
        self.member_profile_impl(community_id, pseudonym_hex)
    }

    fn list_member_profiles(&self, community_id: &str) -> HashMap<String, MemberMentionView> {
        self.list_member_profiles_impl(community_id)
    }

    fn community_roles(&self, community_id: &str) -> Vec<RoleSnapshot> {
        self.community_roles_impl(community_id)
    }

    fn compute_my_permissions(&self, community_id: &str) -> u64 {
        self.compute_my_permissions_impl(community_id)
    }

    // ---------- Community / channel state (mutation) ----------

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64 {
        self.next_sequence_impl(community_id, channel_id)
    }

    fn next_thread_sequence(&self, community_id: &str) -> u64 {
        self.next_sequence_impl(community_id, THREAD_SEQUENCE)
    }

    fn mark_last_send_at(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        self.mark_last_send_at_impl(community_id, channel_id, now_ms);
    }

    fn increment_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        self.increment_lamport_impl(community_id)
    }

    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        self.gov.next_governance_lamport(community_id)
    }

    // ---------- DHT ----------

    async fn write_channel_message_smpl(
        &self,
        context: &ChannelWriteContext,
        channel_msg: &ChannelMessage,
    ) -> Result<DhtWrite, ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_message(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            channel_msg,
        )
        .await;
        delivery(written, "channel write")
    }

    async fn write_channel_forward_smpl(
        &self,
        context: &ChannelWriteContext,
        forward: &ChannelForward,
    ) -> Result<DhtWrite, ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_forward(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            forward,
        )
        .await;
        delivery(written, "channel forward write")
    }

    async fn write_member_reaction_smpl(
        &self,
        context: &ChannelWriteContext,
        reaction: &ChannelReaction,
    ) -> Result<(), ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_reaction(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            reaction,
        )
        .await;
        delivery(written, "reaction write").map(drop)
    }

    async fn write_channel_poll_create_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollCreate,
    ) -> Result<(), ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_poll_create(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            entry,
        )
        .await;
        delivery(written, "poll create write").map(drop)
    }

    async fn write_channel_poll_vote_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollVote,
    ) -> Result<(), ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_poll_vote(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            entry,
        )
        .await;
        delivery(written, "poll vote write").map(drop)
    }

    async fn write_channel_poll_close_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelPollClose,
    ) -> Result<(), ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_poll_close(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            entry,
        )
        .await;
        delivery(written, "poll close write").map(drop)
    }

    async fn write_channel_hand_raise_smpl(
        &self,
        context: &ChannelWriteContext,
        entry: &ChannelHandRaise,
    ) -> Result<(), ChannelError> {
        let w = self.writer(context)?;
        let written = record::write_member_hand_raise(
            &*w.pool,
            &context.channel_key,
            context.slot_index,
            &context.slot_keypair_str,
            w.author,
            &w.signing_key,
            entry,
        )
        .await;
        delivery(written, "hand raise write").map(drop)
    }

    async fn stage_pseudonyms_by_subkey(
        &self,
        community_id: &str,
    ) -> Result<HashMap<u32, String>, ChannelError> {
        Ok(self.stage_pseudonyms_impl(community_id))
    }

    async fn create_smpl_thread_record(
        &self,
        slot_seed_bytes: &[u8; 32],
    ) -> Result<(LeaseId, String), ChannelError> {
        self.create_thread_record_impl(slot_seed_bytes).await
    }

    async fn release_record(&self, lease: LeaseId) {
        self.gov.release_record(lease).await;
    }

    async fn read_all_channel_entries(
        &self,
        community_id: &str,
        record_key: &str,
    ) -> Result<Vec<ChannelEntryItem>, ChannelError> {
        self.read_entries_impl(community_id, record_key).await
    }

    async fn read_all_channel_messages(
        &self,
        community_id: &str,
        record_key: &str,
    ) -> Result<Vec<ChannelMessage>, ChannelError> {
        self.read_messages_impl(community_id, record_key).await
    }

    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases) {
        self.gov.community_records_ready(community_id, leases).await;
    }

    async fn ensure_channel_segment_record(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<String, ChannelError> {
        rekindle_governance_runtime::ensure_channel_segment_record(
            &self.gov,
            community_id,
            channel_id,
        )
        .await
        .map_err(adapter)
    }

    // ---------- The outbox's channel_post stream (plan E2.5) ----------

    async fn queue_post(&self, post: &ChannelPost) -> Result<(), ChannelError> {
        self.queue_post_impl(post).await
    }

    async fn settle_posts(&self, record_key: &str, subkey: u32, landed: bool) -> Vec<ChannelPost> {
        self.settle_posts_impl(record_key, subkey, landed).await
    }

    async fn fail_post(&self, message: &str) {
        self.fail_post_impl(message).await;
    }

    // ---------- DB (channel messages) ----------

    async fn persist_sent_message(
        &self,
        _community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
    ) -> Result<(), ChannelError> {
        self.persist_own_post(channel_id, outcome, body, None).await
    }

    async fn persist_forwarded_message(
        &self,
        _community_id: &str,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
        original_author_pseudonym: &str,
    ) -> Result<(), ChannelError> {
        (self.persist_own_post(channel_id, outcome, body, Some(original_author_pseudonym))).await
    }

    async fn persist_channel_sequence(
        &self,
        community_id: &str,
        channel_id: &str,
        sequence: u64,
    ) -> Result<(), ChannelError> {
        (self.persist_sequence_impl(community_id, channel_id, sequence)).await
    }

    async fn persist_slowmode_state(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), ChannelError> {
        (self.persist_slowmode_impl(community_id, channel_id, now_ms)).await
    }

    async fn find_channel_message_by_id(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<ChannelMessageRow> {
        self.find_message_impl(channel_id, message_id).await
    }

    // ---------- DB (threads) ----------

    async fn persist_thread_row(
        &self,
        _community_id: &str,
        _thread: &ThreadInfoSnapshot,
    ) -> Result<(), ChannelError> {
        Err(not_ported("thread rows"))
    }

    async fn load_thread_metadata(
        &self,
        _community_id: &str,
        _thread_id: &str,
    ) -> Option<ThreadInfoSnapshot> {
        None
    }

    // ---------- Expressions / Mesh / Governance / Permissions ----------

    fn upload_expression_to_cache(
        &self,
        _community_id: &str,
        _expression_id: [u8; 16],
        _bytes: &[u8],
        _filename: String,
        _mime_type: String,
    ) -> Result<AttachmentOffer, ChannelError> {
        Err(not_ported("expression blobs"))
    }

    fn read_expression_bytes(
        &self,
        _community_id: &str,
        _offer: &AttachmentOffer,
    ) -> Option<Vec<u8>> {
        None
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), ChannelError> {
        self.gov.send_to_mesh(community_id, envelope).map_err(adapter)
    }

    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: GovernanceEntry,
    ) -> Result<(), ChannelError> {
        rekindle_governance_runtime::apply::write_entry(&self.gov, community_id, entry)
            .await
            .map_err(adapter)
    }

    fn require_channel_permission(
        &self,
        community_id: &str,
        _channel_id: Option<&str>,
        perm_bits: u64,
    ) -> Result<(), ChannelError> {
        (self.gov.require_permission(community_id, perm_bits))
            .map_err(|e| ChannelError::PermissionDenied(e.to_string()))
    }

    // ---------- Events ----------

    fn emit_event(&self, event: ChannelEvent) {
        Self::emit_event_impl(&event);
    }

    fn emit_chat_event_local(&self, echo: &SentChannelMessageEcho) {
        self.echo_impl(echo);
    }

    fn emit_delivery_succeeded(&self, community_id: &str, channel_id: &str, message_id: &str) {
        self.delivery_impl(community_id, channel_id, message_id, true);
    }

    fn emit_delivery_failed(&self, community_id: &str, channel_id: &str, message_id: &str) {
        self.delivery_impl(community_id, channel_id, message_id, false);
    }
}
