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
//!
//! Every channel of a community appends to the one outbox chain of our
//! member record, but each channel is its own stream, drained on its own
//! (`rekindle_outbox` runs a stream's drain per stream). Two drains of one
//! chain would read the same tip and write the same chain seq, and the
//! later head would replace the earlier: a post lost while its row reads
//! sent (Mac FX3, 2026-11-24). So one drain at a time holds a chain, from
//! its tip read through its head write ([`ChainLocks`]), and the chain
//! refuses a stale append (`chain::append_exact`'s `SeqMoved`) should any
//! writer miss the lock.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_channel::outbox_chain::OutboxWriter;
use rekindle_channel::outbox_send::{drain_posts, DrainOutcome, DrainPost};
use rekindle_channel::poll_id_of;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_outbox::{AttemptOutcome, OutboxRow};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::PollRef;

use super::HostChannel;
use crate::host::adapters::governance::removal_posts::REMOVED_NOT_DELIVERED;
use crate::host::adapters::outbox::{post_of_row, StreamSender};

/// One lock per outbox chain we write, by member record key: a community's
/// drains, whatever their channel, append to it one at a time.
#[derive(Default)]
struct ChainLocks(parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl ChainLocks {
    /// The lock of the chain headed in `member_record`.
    fn of(&self, member_record: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(self.0.lock().entry(member_record.to_owned()).or_default())
    }
}

/// The `channel_post` sender over the Host's channel adapter: one sender
/// serves every channel's stream, so its locks cover all of them.
pub(crate) struct ChannelPostSender {
    channel: HostChannel,
    chains: ChainLocks,
}

impl ChannelPostSender {
    /// The sender over `channel`.
    pub(crate) fn new(channel: HostChannel) -> Self {
        Self {
            channel,
            chains: ChainLocks::default(),
        }
    }
}

#[async_trait]
impl StreamSender for ChannelPostSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        (self.send_stream(std::slice::from_ref(row)).await)
            .into_iter()
            .next()
            .unwrap_or(AttemptOutcome::Deferred)
    }

    async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        self.channel.drain_stream(&self.chains, rows).await
    }
}

impl HostChannel {
    /// Drain `rows`, one `channel_post` stream's due rows in queue order:
    /// the outcomes of a prefix.
    async fn drain_stream(&self, chains: &ChainLocks, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
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
        // Held from the tip read to the head write: another channel's drain
        // on this chain reads the head this one wrote.
        let chain = chains.of(&own.member_record);
        let one_append = chain.lock().await;
        let now = self.ctx().clock.now_ms();
        let drained = drain_posts(self, &writer, &posts, now).await;
        drop(one_append);
        let landed = drained.iter().any(|d| matches!(d, DrainOutcome::Sent(_)));
        let outcomes = (posts.iter().zip(&drained))
            .map(|(item, outcome)| self.outcome_of(&item.post, &own.author, outcome))
            .collect();
        if landed {
            self.gov.hint_own_outbox(&community, &own).await;
        }
        outcomes
    }

    fn outcome_of(
        &self,
        post: &rekindle_types::message::ChannelPost,
        author: &SigningKey,
        outcome: &DrainOutcome,
    ) -> AttemptOutcome {
        // An op on other rows (an edit, a delete, a possession entry, a
        // reaction, a poll op, a hand raise) has no row of its own to mark:
        // its row id is not a message.
        let change = !post.entry.has_row();
        match outcome {
            DrainOutcome::Sent(seq) => {
                tracing::info!(message = %post.message, chain_seq = seq, "channel post on our outbox");
                if !change {
                    self.emit_delivery_succeeded(&post.community, &post.channel, &post.message);
                }
                // The drained post is ours: a create's poll is derived from
                // our pseudonym in this community and its nonce.
                let poll_id = post.entry.poll_ref().map(|poll| match poll {
                    PollRef::Id(id) => *id,
                    PollRef::Created { nonce } => {
                        poll_id_of(&PseudonymKey(author.verifying_key().to_bytes()), nonce)
                    }
                });
                if let Some(poll_id) = poll_id {
                    self.poll_changed(&post.community, &post.channel, &poll_id);
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
