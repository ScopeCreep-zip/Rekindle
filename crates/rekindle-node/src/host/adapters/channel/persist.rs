//! Our stored channel rows (`rekindle_db::repo::messages::channel`), the
//! channel's sequence and slowmode rows (`repo::channels`), and the posts'
//! rows on the outbox's `channel_post` stream (plan E2.5).

use rekindle_channel::deps::{ChannelMessageRow, ChannelPost, ChannelSendOutcome};
use rekindle_channel::error::ChannelError;
use rekindle_db::repo::messages::channel::{self as rows, ChannelMessageInsert};
use rekindle_outbox::OutboxError;
use rekindle_types::message::SendState;

use super::HostChannel;
use crate::host::adapters::outbox::ChannelPosts;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

fn outbox(e: &OutboxError) -> ChannelError {
    ChannelError::Adapter(format!("outbox: {e}"))
}

impl HostChannel {
    /// Our own post's row, stored `Queued` before its record write: its
    /// outbox `channel_post` row moves it from then on (plan E2.5).
    pub(super) async fn persist_own_post(
        &self,
        channel_id: &str,
        outcome: &ChannelSendOutcome,
        body: &str,
        forwarded_from: Option<&str>,
    ) -> Result<(), ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let (channel, body, outcome) = (channel_id.to_string(), body.to_string(), outcome.clone());
        let forwarded = forwarded_from.map(str::to_string);
        db.call(move |conn| {
            rows::insert(
                conn,
                &ChannelMessageInsert {
                    reply_to: outcome.reply_to.as_deref(),
                    owner_key: &owner,
                    channel_id: &channel,
                    sender_key: &outcome.sender_pseudonym_hex,
                    body: &body,
                    timestamp: outcome.timestamp_ms,
                    is_read: true,
                    mek_generation: Some(i64::try_from(outcome.mek_generation).unwrap_or(i64::MAX)),
                    message_id: &outcome.message_id,
                    lamport_ts: outcome.lamport_ts,
                    automod_blurred: false,
                    forwarded_from_author: forwarded.as_deref(),
                    flags: 0,
                    attachment_json: None,
                    send_state: SendState::Queued,
                    mentions_me: false,
                },
            )
        })
        .await
        .map_err(stored)
    }

    pub(super) async fn persist_sequence_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        sequence: u64,
    ) -> Result<(), ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let (community, channel) = (community_id.to_string(), channel_id.to_string());
        let sequence = i64::try_from(sequence).unwrap_or(i64::MAX);
        db.call(move |conn| {
            rekindle_db::repo::channels::set_my_sequence(
                conn, &owner, &community, &channel, sequence,
            )
        })
        .await
        .map_err(stored)
    }

    pub(super) async fn persist_slowmode_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: i64,
    ) -> Result<(), ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let (community, channel) = (community_id.to_string(), channel_id.to_string());
        db.call(move |conn| {
            rekindle_db::repo::channels::set_last_send(conn, &owner, &community, &channel, now_ms)
        })
        .await
        .map_err(stored)
    }

    pub(super) async fn find_message_impl(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<ChannelMessageRow> {
        let (owner, db) = self.owner_and_db().ok()?;
        let (channel, message) = (channel_id.to_string(), message_id.to_string());
        let row = db
            .call(move |conn| rows::find_by_message_id(conn, &owner, &channel, &message))
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "channel message lookup failed"))
            .ok()
            .flatten()?;
        Some(ChannelMessageRow {
            sender_key: row.sender_key,
            body: row.body,
            reply_to: row.reply_to,
        })
    }

    fn channel_posts(&self) -> Result<ChannelPosts, ChannelError> {
        (self.ctx().outbox.channel_posts())
            .ok_or_else(|| ChannelError::Adapter("no unlocked outbox".into()))
    }

    pub(super) async fn queue_post_impl(&self, post: &ChannelPost) -> Result<(), ChannelError> {
        self.channel_posts()?
            .queue(post)
            .await
            .map_err(|e| outbox(&e))
    }

    pub(super) async fn settle_posts_impl(
        &self,
        record_key: &str,
        subkey: u32,
        landed: bool,
    ) -> Vec<ChannelPost> {
        let settled = match self.channel_posts() {
            Ok(posts) => posts.settle(record_key, subkey, landed).await,
            Err(error) => {
                tracing::warn!(%error, "channel posts not settled");
                return Vec::new();
            }
        };
        settled.unwrap_or_else(|error| {
            tracing::warn!(%error, "settling the channel posts failed");
            Vec::new()
        })
    }

    pub(super) async fn fail_post_impl(&self, message: &str) {
        let failed = match self.channel_posts() {
            Ok(posts) => posts.fail(message).await.map(drop).map_err(|e| outbox(&e)),
            Err(error) => Err(error),
        };
        if let Err(error) = failed {
            tracing::warn!(%error, message, "failing the channel post failed");
        }
    }
}
