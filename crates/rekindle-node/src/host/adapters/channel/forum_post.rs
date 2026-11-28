//! Author a forum thread and its starter as one durable operation.

use rekindle_channel::error::ChannelError;
use rekindle_channel::{ChannelMessagingDeps as _, ForumPost};
use rekindle_governance_runtime::post_cut::capture_post_cut;
use rekindle_governance_runtime::{apply::write_forum_thread, GovernanceRuntimeError};
use rekindle_types::dto::community_capability::ChannelCapability;
use rekindle_types::thread::{thread_reply_key, ForumPostCreated};

use super::HostChannel;

impl HostChannel {
    /// Open `post` in `community_id`.
    ///
    /// # Errors
    /// Validation, permission, or the atomic commit failed.
    pub(crate) async fn create_forum_post(
        &self,
        community_id: &str,
        post: ForumPost<'_>,
    ) -> Result<ForumPostCreated, ChannelError> {
        self.require_thread_capability(
            community_id,
            post.channel_id,
            ChannelCapability::CreatePublicThreads,
        )?;
        self.require_thread_capability(
            community_id,
            post.channel_id,
            ChannelCapability::SendInThreads,
        )?;
        let ops = self
            .ctx()
            .outbox
            .governance_ops()
            .ok_or_else(|| ChannelError::Adapter("no unlocked outbox".into()))?;
        let body = post.body;
        let (thread, mut starter, prepared) =
            rekindle_channel::prepare_forum_post(self, community_id, post)?;
        let thread_id = thread.row.id.clone();
        let metadata = super::rich::thread_info(thread.row);
        let channel = self.clone();
        let commit = write_forum_thread(&self.gov, community_id, thread.entry, |op| {
            Box::pin(async move {
                // The signing pipeline holds the rebuild lock here. Check
                // the starter's permission against the state used to sign.
                channel
                    .require_thread_capability(
                        &starter.community,
                        &metadata.channel_id,
                        ChannelCapability::SendInThreads,
                    )
                    .map_err(|error| match error {
                        ChannelError::PermissionDenied(reason) => {
                            GovernanceRuntimeError::ForumStarterRefused(reason)
                        }
                        ChannelError::Removed { banned } => {
                            GovernanceRuntimeError::ForumStarterRemoved { banned }
                        }
                        other => GovernanceRuntimeError::Adapter(other.to_string()),
                    })?;
                starter.governance_cut =
                    capture_post_cut(&channel.gov, &starter.community, Some(&op)).await?;
                ops.queue_forum_thread(op, starter, metadata)
                    .await
                    .map_err(GovernanceRuntimeError::Adapter)
            })
        });
        let commit = async {
            commit.await.map_err(|error| match error {
                GovernanceRuntimeError::PermissionDenied => {
                    super::thread_refusal::thread_create_refused()
                }
                GovernanceRuntimeError::ForumStarterRefused(reason) => {
                    ChannelError::PermissionDenied(reason)
                }
                GovernanceRuntimeError::ForumStarterRemoved { banned } => {
                    ChannelError::Removed { banned }
                }
                _ => ChannelError::Adapter(error.to_string()),
            })
        };
        let sent = rekindle_channel::send_prepared_reply(prepared, commit, |notice| {
            self.send_to_mesh(community_id, notice)
        })
        .await?;
        let created = ForumPostCreated {
            thread_id,
            starter_message_id: thread_reply_key(&sent.sender_pseudonym, &sent.message_id),
            starter: sent.send_state,
        };
        self.publish_own_reply(community_id, &created.thread_id, body, sent)
            .await;
        Ok(created)
    }
}
