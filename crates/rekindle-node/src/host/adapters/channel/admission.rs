//! Authorization of signed outbox posts against their exact governance cut.

use rekindle_channel::admission::authorize_post;
use rekindle_channel::error::ChannelError;
use rekindle_channel::send::BodyPosition;
use rekindle_channel::threads::verified_reply_view;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_db::repo::messages::channel;
use rekindle_db::repo::messages::channel_admission::{self, Post, PostStatus};
use rekindle_governance_runtime::post_cut::resolve_post_cut;
use rekindle_sync::stream::Held;
use rekindle_types::dto::channel_message::{ChannelMessageView, PostAdmission, PostRefusal};
use rekindle_types::governance::OpRef;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::history::{placeholder_view, stored_view};
use super::rich::message_view;
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
    ) -> Result<Option<PostStatus>, ChannelError> {
        let refused = |reason| PostStatus::Refused { reason };
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
            return Ok(Some(PostStatus::Pending));
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
        parent: Option<&str>,
    ) -> Result<Option<Held>, ChannelError> {
        let judge = || {
            self.post_admission(
                &post.community_id,
                &post.channel_id,
                author,
                cut,
                parent.is_some(),
                post.message.timestamp,
            )
        };
        if self
            .blocked_members(&post.community_id)
            .await?
            .contains(&post.message.sender_pseudonym)
        {
            self.discard_post_admission(post, record).await?;
            return Ok(Some(Held::Stored));
        }
        let Some(status) = judge().await? else {
            return Ok(None);
        };
        let changed = self
            .set_post_admission(post, author, record, Some(status), parent)
            .await?;
        if changed && status == PostStatus::Pending {
            let checked = judge().await?;
            if let Some(PostStatus::Refused { .. }) = checked {
                self.set_post_admission(post, author, record, checked, parent)
                    .await?;
            }
            return Ok(checked.map(hold_for_status));
        }
        Ok(Some(hold_for_status(status)))
    }

    pub(super) async fn discard_post_admission(
        &self,
        post: &ReceivedPost,
        record: &str,
    ) -> Result<(), ChannelError> {
        self.write_post_admission(post, record, None, false).await?;
        Ok(())
    }

    pub(super) async fn set_post_admission(
        &self,
        post: &ReceivedPost,
        author: &PseudonymKey,
        record: &str,
        status: Option<PostStatus>,
        parent: Option<&str>,
    ) -> Result<bool, ChannelError> {
        let community = &post.community_id;
        let stream = &post.channel_id;
        let may_read = || match parent {
            Some(parent) => self.require(community, parent, permissions::VIEW_CHANNELS),
            None => {
                self.require_channel_permission(community, Some(stream), permissions::READ_HISTORY)
            }
        };
        let readable = may_read().is_ok();
        let (changed, channel_row) = self
            .write_post_admission(post, record, status, readable && parent.is_none())
            .await?;
        if !changed || !readable {
            return Ok(changed);
        }
        let mut row = if let Some(parent) = parent {
            let parent = ChannelId::from_hex(parent)
                .ok_or_else(|| ChannelError::InvalidId(parent.to_owned()))?;
            let scope = self.keys().scope_for_text(community, parent);
            let row = verified_reply_view(
                self,
                scope,
                BodyPosition {
                    anchor: community,
                    channel_id: stream,
                    author: &author.0,
                    stream_seq: post.body_at().stream_seq,
                },
                &post.message,
                status.map(admission_view),
                self.my_pseudonym_impl(community).as_ref() == Some(&post.message.sender_pseudonym),
            );
            message_view(row)
        } else if let Some(row) = channel_row {
            row
        } else {
            // Some causally admitted posts have no visible row after the ingest gate.
            return Ok(changed);
        };
        if parent.is_none() && row.admission.is_none() {
            let attachments = self.channel_attachments(community, stream).await;
            self.attach_history(community, &attachments, &mut row);
        }
        // In particular, permissions may have changed during the attachment read.
        if may_read().is_ok() {
            let message_id = row.server_message_id.clone().ok_or_else(|| {
                ChannelError::Adapter("a judged post row has no message id".into())
            })?;
            self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::PostAdmissionChanged {
                    community: community.clone(),
                    stream_id: stream.clone(),
                    thread: parent.is_some(),
                    message_id,
                    pending: matches!(row.admission, Some(PostAdmission::Pending { .. })),
                    row: Box::new(row),
                },
            ));
        }
        Ok(changed)
    }

    async fn write_post_admission(
        &self,
        post: &ReceivedPost,
        record: &str,
        status: Option<PostStatus>,
        read_channel_row: bool,
    ) -> Result<(bool, Option<ChannelMessageView>), ChannelError> {
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
        let me = self.my_pseudonym_impl(&community).unwrap_or_default();
        db.call(move |conn| {
            let post = Post {
                owner: &owner,
                community: &community,
                stream: &stream,
                sender: &sender,
                message: &message,
                record: &record,
                timestamp,
            };
            let tx = conn.transaction()?;
            let changed = channel_admission::set(&tx, post, status)?;
            let row = if changed && read_channel_row {
                // History prefers the stored message when both rows exist.
                if let Some(row) = channel::visible_post(&tx, post)? {
                    Some(stored_view(row, &owner, &me))
                } else {
                    channel_admission::get(&tx, post)?.map(placeholder_view)
                }
            } else {
                None
            };
            tx.commit()?;
            Ok((changed, row))
        })
        .await
        .map_err(|e| ChannelError::Adapter(e.to_string()))
    }
}

pub(super) fn admission_view(status: PostStatus) -> PostAdmission {
    match status {
        PostStatus::Pending => PostAdmission::Pending {
            text: "Waiting to verify that this post was allowed.".to_owned(),
        },
        PostStatus::Refused { reason } => reason.into(),
    }
}

fn hold_for_status(status: PostStatus) -> Held {
    match status {
        PostStatus::Pending => Held::Pending,
        PostStatus::Refused { .. } => Held::Stored,
    }
}
