//! The Host's `rekindle_files::FilesDeps` (plan E3.1 E3d), on both modes:
//! the attachment upload, download and expression fetch run over the
//! Host's community store, record pool, outbox, transport and governance
//! adapter. Delegation only: the channel half is [`HostChannel`]'s, the
//! governance half [`HostGovernance`]'s, the rows `rows`'.

use std::path::Path;

use async_trait::async_trait;
use rekindle_channel::deps::{ChannelMessagingDeps, ChannelWriteContext};
use rekindle_codec::community::channel_record::{
    ChannelAttachmentCached, ChannelMessage, ChannelRecordEntry,
};
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_files::{ChannelPost, ChannelWrite, FilesDeps, FilesError, FilesEvent};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_protocol::dht::community::channel_record::{self as record, AppendOutcome};
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};
use uuid::Uuid;

use super::HostFiles;

fn transport(e: impl std::fmt::Display) -> FilesError {
    FilesError::Transport(e.to_string())
}

/// The channel subkeys a member slot can hold.
const CHANNEL_SLOTS: u32 = 255;

impl HostFiles {
    fn membership(
        &self,
        community_id: &str,
    ) -> Result<rekindle_governance_runtime::deps::CommunityMembership, FilesError> {
        (self.gov.community_membership(community_id))
            .ok_or_else(|| FilesError::NotFound(format!("community {community_id}")))
    }

    /// Our slot of `channel_log_key`, for a write of it.
    fn write_context(
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
    ) -> ChannelWriteContext {
        ChannelWriteContext {
            community_id: community_id.to_string(),
            channel_id: String::new(),
            channel_key: channel_log_key.to_string(),
            slot_keypair_str: slot_keypair.to_string(),
            slot_index,
            segment_index: 0,
        }
    }
}

#[async_trait]
impl FilesDeps for HostFiles {
    fn clock(&self) -> std::sync::Arc<dyn rekindle_utils::Clock> {
        GovernanceRuntimeDeps::clock(&self.gov)
    }

    fn rng(&self) -> std::sync::Arc<dyn rekindle_utils::RandomSource> {
        GovernanceRuntimeDeps::rng(&self.gov)
    }

    fn owner_key(&self) -> Result<String, FilesError> {
        self.channel
            .owner_key()
            .ok_or(FilesError::IdentityNotLoaded)
    }

    fn my_pseudonym(&self, community_id: &str) -> Result<String, FilesError> {
        (self.channel.my_pseudonym_hex(community_id))
            .ok_or_else(|| FilesError::NotFound(format!("pseudonym for {community_id}")))
    }

    fn channel_log_key(&self, community_id: &str, channel_id: &str) -> Result<String, FilesError> {
        (self.channel.channel_record_key(community_id, channel_id)).ok_or_else(|| {
            FilesError::NotFound(format!("channel_log_key {community_id}/{channel_id}"))
        })
    }

    fn slot_keypair(&self, community_id: &str) -> Result<String, FilesError> {
        (self.membership(community_id)?.slot_keypair)
            .ok_or_else(|| FilesError::NotFound(format!("slot_keypair {community_id}")))
    }

    fn my_subkey_index(&self, community_id: &str) -> Result<u32, FilesError> {
        (self.membership(community_id)?.my_subkey_index)
            .ok_or_else(|| FilesError::NotFound(format!("my_subkey_index {community_id}")))
    }

    fn channel_is_forum(&self, community_id: &str, channel_id: &str) -> bool {
        (self.channel.channel_info(community_id, channel_id)).is_some_and(|c| c.is_forum)
    }

    fn require_permission(&self, community_id: &str, permission: u64) -> Result<(), FilesError> {
        (self
            .channel
            .require_channel_permission(community_id, None, permission))
        .map_err(|e| FilesError::PermissionDenied(e.to_string()))
    }

    fn enforce_slowmode(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), FilesError> {
        rekindle_channel::pipeline::enforce_slowmode_with_bypass(
            &self.channel,
            community_id,
            channel_id,
            now_ms,
        )
        .map_err(|e| FilesError::Slowmode(e.to_string()))
    }

    fn resolve_outbound_mentions(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        body: &str,
    ) -> (Vec<String>, Vec<String>, u32) {
        rekindle_channel::mentions::resolve_outbound_mentions(
            &self.channel,
            community_id,
            sender_pseudonym,
            body,
        )
    }

    fn increment_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        self.channel.increment_lamport(community_id)
    }

    fn next_channel_sequence(&self, community_id: &str, channel_id: &str) -> u64 {
        self.channel.next_channel_sequence(community_id, channel_id)
    }

    fn peer_route_blob(&self, community_id: &str, target_pseudonym: &str) -> Option<Vec<u8>> {
        (self.gov.online_members(community_id).into_iter())
            .find(|m| m.pseudonym_hex == target_pseudonym)
            .map(|m| m.route_blob)
    }

    fn online_member_pseudonyms(&self, community_id: &str) -> Vec<String> {
        let mut members: Vec<String> = (self.gov.online_members(community_id).into_iter())
            .map(|m| m.pseudonym_hex)
            .collect();
        members.sort_unstable();
        members
    }

    fn governance_pinned_attachments(&self, community_id: &str) -> Vec<Uuid> {
        let mut pinned: Vec<Uuid> = (self.gov.governance_state(community_id))
            .map(|gov| {
                gov.pinned_attachments
                    .iter()
                    .map(|b| Uuid::from_bytes(*b))
                    .collect()
            })
            .unwrap_or_default();
        pinned.sort_unstable();
        pinned
    }

    fn governance_expressions_with_attachments(
        &self,
        community_id: &str,
    ) -> Vec<([u8; 16], AttachmentOffer)> {
        let mut expressions: Vec<([u8; 16], AttachmentOffer)> =
            (self.gov.governance_state(community_id))
                .map(|gov| {
                    (gov.expressions.iter())
                        .filter_map(|(id, expr)| expr.attachment.clone().map(|a| (*id, a)))
                        .collect()
                })
                .unwrap_or_default();
        expressions.sort_unstable_by_key(|(id, _)| *id);
        expressions
    }

    async fn write_channel_message_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        message: &ChannelMessage,
    ) -> Result<ChannelWrite, FilesError> {
        let context = Self::write_context(community_id, channel_log_key, slot_index, slot_keypair);
        let written = self
            .channel
            .write_channel_message_smpl(&context, message)
            .await;
        match written.map_err(transport)? {
            rekindle_channel::deps::DhtWrite::Stored => Ok(ChannelWrite::Stored),
            rekindle_channel::deps::DhtWrite::Held => Ok(ChannelWrite::Held),
        }
    }

    async fn queue_post(&self, post: &ChannelPost) -> Result<(), FilesError> {
        (self.channel.queue_post(post).await).map_err(|e| FilesError::Db(e.to_string()))
    }

    async fn settle_posts(&self, record_key: &str, subkey: u32) {
        self.channel.settle_posts(record_key, subkey, true).await;
    }

    async fn fail_post(&self, message_id: &str) {
        self.channel.fail_post(message_id).await;
    }

    async fn write_attachment_cached_to_smpl(
        &self,
        community_id: &str,
        channel_log_key: &str,
        slot_index: u32,
        slot_keypair: &str,
        cached: &ChannelAttachmentCached,
    ) -> Result<(), FilesError> {
        let credentials = (self.channel.pseudonym_credentials(community_id)).map_err(transport)?;
        let written = record::write_member_attachment_cached(
            &*self.records()?,
            channel_log_key,
            slot_index,
            slot_keypair,
            credentials.pseudonym,
            &credentials.signing_key,
            cached,
        )
        .await;
        // A held page lands when it can (plan C7.13); an `AttachmentCached`
        // entry carries no delivery state, so held is written.
        match written {
            Ok(AppendOutcome::Stored | AppendOutcome::Held)
            | Err(rekindle_protocol::ProtocolError::PoolClosed) => Ok(()),
            Err(e) => Err(transport(format!("AttachmentCached write: {e}"))),
        }
    }

    async fn scan_channel_subkeys(
        &self,
        channel_log_key: &str,
    ) -> Result<Vec<ChannelRecordEntry>, FilesError> {
        let pool = self.records()?;
        // One borrow, one inspect, and a read of only the subkeys that hold
        // a value (plan C9.1e).
        let lease = (pool.acquire(channel_log_key, None).await)
            .map_err(|e| transport(format!("open channel record: {e}")))?;
        let subkeys: Vec<u32> = (0..CHANNEL_SLOTS).collect();
        let values = pool.read_changed(lease, &subkeys).await;
        pool.release(lease).await;
        let values = values.map_err(|e| transport(format!("read channel record: {e}")))?;
        Ok((values.into_iter())
            .filter_map(|(_, value)| {
                rekindle_codec::community::channel_record::decode_channel_entries(&value.data).ok()
            })
            .flatten()
            .collect())
    }

    async fn app_call_peer(
        &self,
        route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, FilesError> {
        (self.gov.app_call_peer(route_blob, payload).await).map_err(transport)
    }

    async fn insert_channel_message_full(
        &self,
        row: rekindle_files::InsertChannelMessage<'_>,
    ) -> Result<(), FilesError> {
        self.insert_row(row).await
    }

    fn persist_slowmode_state(&self, community_id: &str, channel_id: &str, now_ms: i64) {
        self.channel
            .mark_last_send_at(community_id, channel_id, now_ms);
        let (channel, community, chan) = (
            self.channel.clone(),
            community_id.to_string(),
            channel_id.to_string(),
        );
        let scope = self.gov.context().unlock_scope_or_closed();
        let stored = scope.spawn("attachment slowmode row", async move {
            if let Err(error) = channel
                .persist_slowmode_state(&community, &chan, now_ms)
                .await
            {
                tracing::warn!(%error, "storing the attachment's slowmode row failed");
            }
        });
        if stored.is_err() {
            tracing::debug!("no unlock scope: the slowmode row is not stored");
        }
    }

    async fn persist_local_path(
        &self,
        owner_key: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        save_path: &Path,
    ) -> Result<(), FilesError> {
        self.store_local_path(owner_key, channel_id, attachment_id_hex, save_path)
            .await
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), FilesError> {
        (self.gov.send_to_mesh(community_id, envelope)).map_err(transport)
    }

    async fn write_attachment_pinned(
        &self,
        community_id: &str,
        attachment_id: [u8; 16],
        pinned: bool,
    ) -> Result<(), FilesError> {
        let entry = GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
        };
        (self
            .channel
            .write_governance_entry(community_id, entry)
            .await)
            .map_err(transport)
    }

    fn emit_event(&self, event: FilesEvent) {
        let event = match event {
            FilesEvent::AttachmentDownloaded {
                community_id,
                channel_id,
                attachment_id_hex,
            } => ChannelMessageEvent::AttachmentDownloaded {
                community: community_id,
                channel: channel_id,
                attachment_id: attachment_id_hex,
            },
            FilesEvent::ExpressionAssetReady {
                community_id,
                expression_id_hex,
            } => ChannelMessageEvent::ExpressionAssetReady {
                community: community_id,
                expression_id: expression_id_hex,
            },
        };
        (self.gov.context()).publish_event(SubscriptionEvent::ChannelMessage(event));
    }
}
