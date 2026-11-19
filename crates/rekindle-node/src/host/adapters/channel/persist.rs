//! Our stored channel rows (`rekindle_db::repo::messages::channel`), the
//! channel's sequence and slowmode rows (`repo::channels`), and the posts'
//! rows on the outbox's `channel_post` stream (plan E2.5).

use rekindle_channel::deps::{ChannelMessageRow, ChannelPost, ChannelSendOutcome};
use rekindle_channel::error::ChannelError;
use rekindle_db::repo::messages::channel::{self as rows, ChannelMessageInsert};
use rekindle_outbox::OutboxError;

use super::HostChannel;
use crate::host::adapters::outbox::ChannelPosts;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

fn outbox(e: &OutboxError) -> ChannelError {
    ChannelError::Adapter(format!("outbox: {e}"))
}

impl HostChannel {
    /// Our own post's row, stored `Queued` before its record write (or
    /// `AwaitingKey` before its channel's key, plan E3.2 S7l): its outbox
    /// `channel_post` row moves it from then on (plan E2.5).
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
                    mek_generation: outcome
                        .mek_generation
                        .map(|g| i64::try_from(g).unwrap_or(i64::MAX)),
                    message_id: &outcome.message_id,
                    // Placed at its stream seq when queued (D22).
                    lamport_ts: 0,
                    automod_blurred: false,
                    forwarded_from_author: forwarded.as_deref(),
                    flags: 0,
                    attachment_json: None,
                    send_state: outcome.send_state,
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

    /// Queue our edit or delete `post` on its channel's stream (no echo:
    /// the change is stored on its target's row).
    /// Queue our change `post`; its stream seq.
    pub(crate) async fn queue_change_impl(&self, post: &ChannelPost) -> Result<u64, ChannelError> {
        self.channel_posts()?
            .queue_change(post)
            .await
            .map_err(|e| outbox(&e))
    }

    pub(super) async fn queue_post_impl(&self, post: &ChannelPost) -> Result<(), ChannelError> {
        // An op on other rows has no `messages` row to echo it.
        let posts = self.channel_posts()?;
        let queued = if post.entry.has_row() {
            posts.queue(post).await
        } else {
            posts.queue_change(post).await.map(drop)
        };
        queued.map_err(|e| outbox(&e))
    }

    pub(super) async fn queue_waiting_post_impl(
        &self,
        post: &ChannelPost,
        deadline_ms: i64,
    ) -> Result<(), ChannelError> {
        self.channel_posts()?
            .queue_waiting(post, deadline_ms)
            .await
            .map_err(|e| outbox(&e))
    }

    pub(super) async fn waiting_posts_impl(&self, community_id: &str) -> Vec<ChannelPost> {
        let waiting = match self.channel_posts() {
            Ok(posts) => posts
                .waiting(Some(community_id))
                .await
                .map_err(|e| outbox(&e)),
            Err(error) => Err(error),
        };
        waiting.unwrap_or_else(|error| {
            tracing::warn!(%error, community = community_id, "waiting channel posts not read");
            Vec::new()
        })
    }

    /// Release the waiting post `message` to its stream; `false` when its
    /// row was not waiting.
    pub(super) async fn release_waiting_post_impl(&self, message: &str) -> bool {
        let moved = match self.channel_posts() {
            Ok(posts) => posts.release(message).await.map_err(|e| outbox(&e)),
            Err(error) => Err(error),
        };
        moved.unwrap_or_else(|error| {
            tracing::warn!(%error, message, "waiting channel post not moved");
            false
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
