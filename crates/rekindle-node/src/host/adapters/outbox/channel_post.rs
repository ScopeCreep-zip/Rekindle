//! The `channel_post` stream (plan E2.5, C7.14): a community channel post
//! is a row of the stream of the record slot its write targets, with the
//! post's `messages` row as its echo. The record pool's write is the send,
//! so the kind has no [`super::StreamSender`] (its rows are `Deferred` on
//! every pass): the pool's settle of the slot settles every row waiting on
//! it, `sent` when our page landed and `failed` when the network
//! superseded it. The rows outlive a lock, as the pool's held write does
//! (a `dht_write` row), so the next unlock's settle still finds them.

use std::sync::Arc;

use rekindle_db::repo::outbox as repo;
use rekindle_outbox::{Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind};
use rekindle_types::message::ChannelPost;

use super::{storage, HostOutbox, OutboxSlot};
use crate::host::runtime::Host;

/// The reason a post whose page the network superseded fails with.
pub const SUPERSEDED: &str = "superseded";

fn scope(record_key: &str, subkey: u32) -> String {
    format!("{record_key}:{subkey}")
}

/// The unlocked identity's `channel_post` stream.
pub struct ChannelPosts(pub(super) Arc<Outbox<HostOutbox>>);

impl ChannelPosts {
    /// Store `post`'s row, its echo the post already written `queued`.
    ///
    /// # Errors
    /// The storage failed, or no queued post is `post.message`.
    pub async fn queue(&self, post: &ChannelPost) -> Result<(), OutboxError> {
        let payload = serde_json::to_vec(post).map_err(|e| storage(&e))?;
        let row = self.0.new_row(Entry {
            kind: StreamKind::ChannelPost,
            scope: scope(&post.record_key, post.subkey),
            client_id: post.message.clone(),
            payload,
            deadline: Deadline::Horizon,
        });
        let (db, owner) = self.0.deps().db()?;
        db.call(move |conn| {
            let new = repo::NewRow {
                owner_key: &owner,
                stream: &row.stream,
                client_id: &row.client_id,
                kind: row.kind.as_str(),
                payload: &row.payload,
                next_at: row.next_at,
                deadline: row.deadline,
            };
            repo::enqueue_for_message(conn, &new, &row.client_id)
        })
        .await
        .map_err(|e| storage(&e))?;
        self.0.wake();
        Ok(())
    }

    /// The write of `record_key`/`subkey` settled: every post waiting on
    /// it is `sent` when it `landed`, else `failed` ([`SUPERSEDED`]).
    /// Returns the posts settled.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn settle(
        &self,
        record_key: &str,
        subkey: u32,
        landed: bool,
    ) -> Result<Vec<ChannelPost>, OutboxError> {
        let outcome = if landed {
            Ok(())
        } else {
            Err(SUPERSEDED.to_owned())
        };
        let rows = (self.0)
            .settle_stream(StreamKind::ChannelPost, &scope(record_key, subkey), outcome)
            .await?;
        Ok(rows.iter().filter_map(post_of).collect())
    }

    /// `message`'s write could not be made: its row settles `failed`.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn fail(&self, message: &str) -> Result<bool, OutboxError> {
        self.0.discard(message).await
    }
}

fn post_of(row: &OutboxRow) -> Option<ChannelPost> {
    match serde_json::from_slice(&row.payload) {
        Ok(post) => Some(post),
        Err(e) => {
            tracing::warn!(error = %e, client_id = %row.client_id, "unreadable channel post row");
            None
        }
    }
}

impl OutboxSlot {
    /// The running outbox's `channel_post` stream.
    pub(crate) fn channel_posts(&self) -> Option<ChannelPosts> {
        self.current.read().clone().map(ChannelPosts)
    }
}

impl Host {
    /// The unlocked identity's `channel_post` stream, for the GUI's channel
    /// send and settle path (`rekindle-channel`'s pipeline, until E3.1).
    #[must_use]
    pub fn channel_posts(&self) -> Option<ChannelPosts> {
        self.ctx.outbox.channel_posts()
    }
}
