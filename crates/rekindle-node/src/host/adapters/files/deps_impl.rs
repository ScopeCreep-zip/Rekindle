//! The Host's `rekindle_files::FilesDeps` (plan E3.1 E3d), on both modes:
//! the attachment upload, download and expression fetch run over the
//! Host's community store, record pool, outbox, transport and governance
//! adapter. Delegation only: the channel half is [`HostChannel`]'s, the
//! governance half [`HostGovernance`]'s, the rows `rows`'.

use std::path::Path;

use async_trait::async_trait;
use rekindle_channel::deps::ChannelMessagingDeps;
use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_files::{ChannelPost, FilesDeps, FilesError, FilesEvent};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};
use uuid::Uuid;

use super::HostFiles;

fn transport(e: impl std::fmt::Display) -> FilesError {
    FilesError::Transport(e.to_string())
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

    /// The member's `M`/6 route from the Host's cache (plan E3.4 S4), on
    /// both modes.
    fn peer_route_blob(&self, community_id: &str, target_pseudonym: &str) -> Option<Vec<u8>> {
        self.gov.cached_member_route(community_id, target_pseudonym)
    }

    /// The members with a cached route whose beat shows them present
    /// (plan E3.4 S4).
    fn online_member_pseudonyms(&self, community_id: &str) -> Vec<String> {
        let mut members: Vec<String> = (self.gov.reachable_members(community_id).into_iter())
            .map(|(pseudonym, _)| pseudonym)
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

    async fn queue_post(&self, post: &ChannelPost) -> Result<(), FilesError> {
        // A possession entry has no `messages` row to echo it
        // (`PostEntry::has_row`).
        (self.channel.queue_post(post).await).map_err(|e| FilesError::Db(e.to_string()))
    }

    async fn channel_entries(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<Vec<ChannelRecordEntry>, FilesError> {
        // A channel has no record (plan E3.3 slice 8): its entries are
        // the members' channel outbox entries, linked by tag.
        Ok((self.channel)
            .read_author_outboxes(community_id, channel_id)
            .await
            .into_iter()
            .map(|item| item.entry)
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
            // The transfer driver is its one subscriber (lead decision 51).
            FilesEvent::ChunkCached {
                attachment_id_hex,
                chunks_done,
                chunk_count,
                bytes_done,
                total,
                ..
            } => {
                crate::host::transfers::chunk_cached(
                    self.gov.context(),
                    &attachment_id_hex,
                    crate::host::transfers::Checkpoint {
                        chunks_done,
                        chunk_count,
                        bytes_done,
                        total,
                    },
                );
                return;
            }
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
