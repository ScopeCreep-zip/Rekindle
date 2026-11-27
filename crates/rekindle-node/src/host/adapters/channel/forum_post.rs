//! Author a forum thread and its starter as one durable operation.

use rekindle_channel::error::ChannelError;
use rekindle_channel::{ChannelMessagingDeps as _, ForumPost};
use rekindle_governance_runtime::{apply::write_forum_thread, GovernanceRuntimeError};
use rekindle_types::permissions;
use rekindle_types::thread::ForumPostCreated;

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
        self.refuse_removed(community_id)?;
        self.require(
            community_id,
            post.channel_id,
            permissions::CREATE_PUBLIC_THREADS,
        )?;
        self.require(community_id, post.channel_id, permissions::SEND_IN_THREADS)?;
        let ops = self
            .ctx()
            .outbox
            .governance_ops()
            .ok_or_else(|| ChannelError::Adapter("no unlocked outbox".into()))?;
        let body = post.body;
        let (thread, starter, prepared) =
            rekindle_channel::prepare_forum_post(self, community_id, post)?;
        let thread_id = thread.row.id.clone();
        let metadata = super::rich::thread_info(thread.row);
        let channel = self.clone();
        let commit = write_forum_thread(&self.gov, community_id, thread.entry, |op| {
            Box::pin(async move {
                // The signing pipeline holds the rebuild lock here. Check
                // the starter's permission against the state used to sign.
                channel
                    .require(
                        &starter.community,
                        &metadata.channel_id,
                        permissions::SEND_IN_THREADS,
                    )
                    .map_err(|_| GovernanceRuntimeError::PermissionDenied)?;
                ops.queue_forum_thread(op, starter, metadata)
                    .await
                    .map_err(GovernanceRuntimeError::Adapter)
            })
        });
        let commit = async {
            commit.await.map_err(|error| match error {
                GovernanceRuntimeError::PermissionDenied => {
                    ChannelError::PermissionDenied(error.to_string())
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
            starter_message_id: sent.message_id.clone(),
            starter: sent.send_state,
        };
        self.publish_own_reply(community_id, &created.thread_id, body, sent)
            .await;
        Ok(created)
    }
}
