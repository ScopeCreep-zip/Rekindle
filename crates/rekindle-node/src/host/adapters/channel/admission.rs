//! Authorization of signed outbox posts against their exact governance cut.

use rekindle_channel::admission::authorize_post;
use rekindle_channel::error::ChannelError;
use rekindle_db::repo::messages::channel_admission::{self, Post};
use rekindle_governance_runtime::post_cut::resolve_post_cut;
use rekindle_sync::stream::Held;
use rekindle_types::dto::channel_message::{PostAdmission, PostRefusal};
use rekindle_types::governance::OpRef;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::{ingest::ReceivedPost, HostChannel};

impl HostChannel {
    pub(super) async fn post_admission(
        &self,
        community: &str,
        stream: &str,
        author: &PseudonymKey,
        cut: &[OpRef],
        thread: bool,
        timestamp: u64,
    ) -> Result<Option<PostAdmission>, ChannelError> {
        let refused = |reason| PostAdmission::Refused { reason };
        if cut.is_empty() {
            return Ok(Some(refused(PostRefusal::MissingCut)));
        }
        let Some(stream) = ChannelId::from_hex(stream) else {
            return Ok(Some(refused(PostRefusal::UnknownStream)));
        };
        let Some(state) = resolve_post_cut(&self.gov, community, cut)
            .await
            .map_err(|e| ChannelError::Adapter(e.to_string()))?
        else {
            return Ok(Some(PostAdmission::Pending));
        };
        Ok(
            authorize_post(&state, author, stream, thread, timestamp / 1000)
                .err()
                .map(refused),
        )
    }

    /// Publish a placeholder before leaving the cursor at an unresolved entry.
    /// Check again when the placeholder is first written so a governance
    /// projection racing that write cannot miss its retry.
    pub(super) async fn record_post_admission(
        &self,
        post: &ReceivedPost,
        author: &PseudonymKey,
        cut: &[OpRef],
        record: &str,
        thread: bool,
    ) -> Result<Option<Held>, ChannelError> {
        let judge = || {
            self.post_admission(
                &post.community_id,
                &post.channel_id,
                author,
                cut,
                thread,
                post.message.timestamp,
            )
        };
        if self
            .blocked_members(&post.community_id)
            .await?
            .contains(&post.message.sender_pseudonym)
        {
            self.set_post_admission(post, record, None, thread).await?;
            return Ok(Some(Held::Stored));
        }
        let Some(status) = judge().await? else {
            return Ok(None);
        };
        let changed = self
            .set_post_admission(post, record, Some(status), thread)
            .await?;
        if changed && status == PostAdmission::Pending {
            let checked = judge().await?;
            if let Some(PostAdmission::Refused { .. }) = checked {
                self.set_post_admission(post, record, checked, thread)
                    .await?;
            }
            return Ok(checked.map(hold_for_status));
        }
        Ok(Some(hold_for_status(status)))
    }

    pub(super) async fn set_post_admission(
        &self,
        post: &ReceivedPost,
        record: &str,
        status: Option<PostAdmission>,
        thread: bool,
    ) -> Result<bool, ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let Some(message) = post.message.message_id.clone() else {
            return Err(ChannelError::Adapter(
                "a received post has no message id".into(),
            ));
        };
        let community = post.community_id.clone();
        let stream = post.channel_id.clone();
        let sender = post.message.sender_pseudonym.clone();
        let record = record.to_owned();
        let timestamp = i64::try_from(post.message.timestamp).unwrap_or(i64::MAX);
        let changed = db
            .call(move |conn| {
                channel_admission::set(
                    conn,
                    Post {
                        owner: &owner,
                        community: &community,
                        stream: &stream,
                        sender: &sender,
                        message: &message,
                        record: &record,
                        timestamp,
                    },
                    status,
                )
            })
            .await
            .map_err(|e| ChannelError::Adapter(e.to_string()))?;
        if changed {
            self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::PostAdmissionChanged {
                    community: post.community_id.clone(),
                    stream_id: post.channel_id.clone(),
                    thread,
                    pending: status == Some(PostAdmission::Pending),
                },
            ));
        }
        Ok(changed)
    }
}

fn hold_for_status(status: PostAdmission) -> Held {
    match status {
        PostAdmission::Pending => Held::Pending,
        PostAdmission::Refused { .. } => Held::Stored,
    }
}
