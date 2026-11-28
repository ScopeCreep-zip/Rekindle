//! The single `ChannelMessagingDeps` impl for the Host. Delegation only:
//! every body is a call into a sibling module, the governance adapter or
//! the shared crates.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_channel::deps::{
    ChannelInfoSnapshot, ChannelMessageRow, ChannelMessagingDeps, ChannelPost, ChannelSendOutcome,
    MemberMentionView, PseudonymCredentials, RoleSnapshot, SentChannelMessageEcho,
    ThreadInfoSnapshot, ThreadStateSnapshot,
};
use rekindle_channel::error::ChannelError;
use rekindle_channel::event::ChannelEvent;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::{GovernanceEntry, OpHash};
use rekindle_types::id::{ChannelId, PseudonymKey};

use super::HostChannel;

fn adapter(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(e.to_string())
}

#[async_trait]
impl ChannelMessagingDeps for HostChannel {
    fn now_secs(&self) -> u64 {
        self.ctx().clock.now_secs()
    }

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

    fn my_role_ids(&self, community_id: &str) -> Vec<rekindle_types::id::RoleId> {
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

    fn mark_last_send_at(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        self.mark_last_send_at_impl(community_id, channel_id, now_ms);
    }

    // ---------- DHT ----------

    async fn release_record(&self, lease: LeaseId) {
        self.gov.release_record(lease).await;
    }

    async fn read_outbox_entries(
        &self,
        community_id: &str,
        stream_id: &str,
    ) -> Vec<rekindle_channel::deps::OutboxItem> {
        self.read_author_outboxes(community_id, stream_id).await
    }

    async fn community_records_ready(&self, community_id: &str, leases: CommunityLeases) {
        self.gov.community_records_ready(community_id, leases).await;
    }

    // ---------- The outbox's channel_post stream (plan E2.5) ----------

    async fn pending_thread_replies(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Result<Vec<rekindle_channel::deps::PendingThreadReply>, ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        super::super::outbox::pending_thread_replies(&db, owner, community_id, thread_id)
            .await
            .map_err(adapter)
    }

    async fn queue_post(&self, post: &ChannelPost) -> Result<(), ChannelError> {
        self.queue_post_impl(post).await
    }

    async fn fail_post(&self, message: &str) {
        self.fail_post_impl(message).await;
    }

    async fn queue_waiting_post(
        &self,
        post: &ChannelPost,
        deadline_ms: i64,
    ) -> Result<(), ChannelError> {
        self.queue_waiting_post_impl(post, deadline_ms).await
    }

    async fn waiting_posts(&self, community_id: &str) -> Vec<ChannelPost> {
        self.waiting_posts_impl(community_id).await
    }

    async fn release_waiting_post(&self, message: &str) -> bool {
        self.release_waiting_post_impl(message).await
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
        community_id: &str,
        thread: &ThreadInfoSnapshot,
    ) -> Result<(), ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let (community, row) = (
            community_id.to_string(),
            super::rich::thread_info(thread.clone()),
        );
        db.call(move |conn| rekindle_db::repo::threads::upsert(conn, &owner, &community, &row))
            .await
            .map_err(adapter)
    }

    async fn load_thread_metadata(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Option<ThreadInfoSnapshot> {
        let (owner, db) = self.owner_and_db().ok()?;
        let (community, thread) = (community_id.to_string(), thread_id.to_string());
        db.call(move |conn| rekindle_db::repo::threads::get(conn, &owner, &community, &thread))
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "thread row read failed"))
            .ok()
            .flatten()
            .map(super::rich::thread_snapshot)
    }

    async fn thread_created_at(
        &self,
        community_id: &str,
        op: &OpHash,
    ) -> Result<u64, ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let community = community_id.to_owned();
        let hash = op.0;
        db.call(move |conn| {
            rekindle_db::repo::governance_ops::get_op(conn, &owner, &community, &hash)
        })
        .await
        .map_err(adapter)?
        .map(|op| op.issued_at)
        .ok_or_else(|| ChannelError::Storage("Thread creation metadata is unavailable.".into()))
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
        rekindle_files::upload_expression_to_cache(
            &self.files(),
            community_id,
            expression_id,
            bytes,
            filename,
            mime_type,
        )
        .map_err(|e| ChannelError::Adapter(format!("expression cache: {e}")))
    }

    fn read_expression_bytes(
        &self,
        community_id: &str,
        offer: &AttachmentOffer,
    ) -> Option<Vec<u8>> {
        rekindle_files::read_expression_bytes(&self.files(), community_id, offer)
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), ChannelError> {
        self.gov
            .send_to_mesh(community_id, envelope)
            .map_err(adapter)
    }

    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: GovernanceEntry,
    ) -> Result<(), ChannelError> {
        if let (Some(gov), Some(me)) = (
            self.gov.governance_state(community_id),
            self.my_pseudonym_impl(community_id),
        ) {
            super::thread_refusal::refuse_timed_out_thread(
                &entry,
                &gov,
                &PseudonymKey::from_hex_lossy(&me),
                self.ctx().clock.now_secs(),
            )?;
        }
        let creates_thread = matches!(entry, GovernanceEntry::ThreadCreated { .. });
        rekindle_governance_runtime::apply::write_entry(&self.gov, community_id, entry)
            .await
            .map(drop)
            .map_err(|e| match e {
                rekindle_governance_runtime::GovernanceRuntimeError::PermissionDenied
                    if creates_thread =>
                {
                    super::thread_refusal::thread_create_refused()
                }
                rekindle_governance_runtime::GovernanceRuntimeError::PermissionDenied => {
                    ChannelError::PermissionDenied(e.to_string())
                }
                other => adapter(other),
            })
    }

    /// A channel action is judged on that channel's effective permissions,
    /// its overwrites applied (architecture §9.2 steps 4-6, plan E3.2
    /// SEC): a channel an overwrite closes to us refuses its history,
    /// sends and moderation even where the community-wide bits allow them.
    /// `None` is a community-wide action.
    fn require_channel_permission(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        perm_bits: u64,
    ) -> Result<(), ChannelError> {
        let channel = channel_id
            .map(|id| {
                ChannelId::from_hex(id)
                    .ok_or_else(|| ChannelError::InvalidId("Invalid channel.".into()))
            })
            .transpose()?;
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        if self.skip_local_post_permission
            && perm_bits == rekindle_types::permissions::SEND_MESSAGES
        {
            return Ok(());
        }
        self.gov
            .require_own_permissions(community_id, channel.as_ref(), perm_bits)
            .map_err(ChannelError::PermissionDenied)
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

    fn emit_send_state(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        state: rekindle_types::message::SendState,
    ) {
        self.send_state_impl(community_id, channel_id, message_id, state, None);
    }
}
