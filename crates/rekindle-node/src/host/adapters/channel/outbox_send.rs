//! The `channel_post` stream's sender (plan E3.3), modelled on the
//! `governance_op` sender: one channel's due rows, in queue order, are
//! drained onto our channel outbox (`M`/4) by
//! [`rekindle_channel::outbox_send::drain_posts`], which seals each body
//! now at its stream seq. A row settles `sent` once the head we published
//! names its chain seq; a post whose channel key we do not hold yet, or a
//! write that missed, is deferred and retried by the stream (this folds in
//! the awaiting-key release and the user's Retry: both queue the row, and
//! the sender writes it when it can). After a drain lands, the author's own
//! `HeadAdvanced` for subkey 4 tells readers to diff our outbox.

use async_trait::async_trait;
use rekindle_channel::outbox_chain::OutboxWriter;
use rekindle_channel::outbox_send::{drain_posts, DrainOutcome, DrainPost};
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_outbox::{AttemptOutcome, OutboxRow};
use rekindle_types::message::PostEntry;

use super::HostChannel;
use crate::host::adapters::governance::removal_posts::REMOVED_NOT_DELIVERED;
use crate::host::adapters::outbox::{post_of_row, StreamSender};

/// The `channel_post` sender over the Host's channel adapter.
pub(crate) struct ChannelPostSender(pub(crate) HostChannel);

#[async_trait]
impl StreamSender for ChannelPostSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        (self.send_stream(std::slice::from_ref(row)).await)
            .into_iter()
            .next()
            .unwrap_or(AttemptOutcome::Deferred)
    }

    async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        self.0.drain_stream(rows).await
    }
}

impl HostChannel {
    /// Drain `rows`, one `channel_post` stream's due rows in queue order:
    /// the outcomes of a prefix.
    async fn drain_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        let mut posts = Vec::with_capacity(rows.len());
        for row in rows {
            match post_of_row(row) {
                Ok(post) => posts.push(DrainPost {
                    post,
                    stream_seq: u64::try_from(row.stream_seq).unwrap_or_default(),
                }),
                Err(reason) if posts.is_empty() => return vec![AttemptOutcome::Wedged(reason)],
                Err(_) => break,
            }
        }
        let Some(community) = posts.first().map(|p| p.post.community.clone()) else {
            return Vec::new();
        };
        // Removed from the community: nothing of ours reaches its readers,
        // and the post reads why (plan E3.2 S9, F9-3).
        if self.refuse_removed(&community).is_err() {
            return posts
                .iter()
                .map(|_| AttemptOutcome::Superseded(REMOVED_NOT_DELIVERED.to_owned()))
                .collect();
        }
        let own = match self.gov.own_outbox(&community) {
            Ok(own) => own,
            Err(error) => {
                tracing::debug!(%community, %error, "channel outbox not writable yet");
                return vec![AttemptOutcome::Deferred];
            }
        };
        let Ok(port) = self.records() else {
            return vec![AttemptOutcome::Deferred];
        };
        let writer = OutboxWriter {
            port: &*port,
            member_record: &own.member_record,
            owner: &own.owner,
            anchor: &community,
            author: &own.author,
        };
        let now = self.ctx().clock.now_ms();
        let drained = drain_posts(self, &writer, &posts, now).await;
        let landed = drained.iter().any(|d| matches!(d, DrainOutcome::Sent(_)));
        let outcomes = (posts.iter().zip(&drained))
            .map(|(item, outcome)| self.outcome_of(&item.post, outcome))
            .collect();
        if landed {
            self.gov.hint_own_outbox(&community, &own).await;
        }
        outcomes
    }

    fn outcome_of(
        &self,
        post: &rekindle_types::message::ChannelPost,
        outcome: &DrainOutcome,
    ) -> AttemptOutcome {
        // An edit, a delete or a possession entry has no row of its own to
        // mark: its row id is not a message.
        let change = matches!(
            post.entry,
            PostEntry::Edit { .. } | PostEntry::Delete { .. } | PostEntry::Cached { .. }
        );
        match outcome {
            DrainOutcome::Sent(seq) => {
                tracing::info!(message = %post.message, chain_seq = seq, "channel post on our outbox");
                if !change {
                    self.emit_delivery_succeeded(&post.community, &post.channel, &post.message);
                }
                AttemptOutcome::Sent
            }
            DrainOutcome::Deferred(reason) => {
                tracing::debug!(message = %post.message, %reason, "channel post deferred");
                AttemptOutcome::Deferred
            }
            DrainOutcome::Failed(reason) => {
                tracing::warn!(message = %post.message, %reason, "channel post cannot be written");
                if !change {
                    self.emit_delivery_failed(&post.community, &post.channel, &post.message);
                }
                AttemptOutcome::Superseded(reason.clone())
            }
        }
    }
}
