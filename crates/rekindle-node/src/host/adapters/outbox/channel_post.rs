//! The `channel_post` stream (plan E3.3, D22): our community channel post
//! is a row of its channel's stream `channel_post:<community>:<channel>`,
//! with the post's `messages` row as its echo. The stream's sender
//! (`HostChannel`'s `ChannelPostSender`, modelled on the `governance_op`
//! sender) drains the due rows in queue order onto our member record's
//! channel outbox (`M`/4); a row settles `sent` when the head we published
//! names its chain seq, and the outbox retries the rest. The user's Retry
//! of a failed post queues the same row again (the engine's retry).
//!
//! A post composed before we held its channel's key waits on its row in
//! state `awaiting_key` (plan E3.2 S7l, F9-1) until the key's install
//! releases it ([`ChannelPosts::release`]), its deadline (one day from
//! composition) fails it, or our departure discards it
//! ([`ChannelPosts::discard_community`]); a lock keeps it. A released or
//! retried post whose key is absent again is deferred by the sender.
//!
//! An attachment message is a post of the same stream (slice 7), its offer
//! on the row.

use std::sync::Arc;

use rekindle_db::repo::outbox as repo;
use rekindle_outbox::{
    Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind, OUTBOX_DEADLINE,
};
use rekindle_types::message::ChannelPost;

use super::{storage, HostOutbox, OutboxSlot};
use crate::host::runtime::Host;

/// The unlocked identity's `channel_post` stream.
pub struct ChannelPosts(pub(super) Arc<Outbox<HostOutbox>>);

/// The post a `channel_post` row carries.
///
/// # Errors
/// The row's payload is not a post.
pub fn post_of_row(row: &OutboxRow) -> Result<ChannelPost, String> {
    serde_json::from_slice(&row.payload).map_err(|e| format!("unreadable channel post row: {e}"))
}

impl ChannelPosts {
    /// Store `post`'s row, its echo the post already written `queued`.
    ///
    /// # Errors
    /// The storage failed, or no queued post is `post.message`.
    pub async fn queue(&self, post: &ChannelPost) -> Result<(), OutboxError> {
        let payload = serde_json::to_vec(post).map_err(|e| storage(&e))?;
        let row = self.0.new_row(Entry {
            kind: StreamKind::ChannelPost,
            scope: post.scope(),
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
                depends_on: None,
            };
            repo::enqueue_for_message(conn, &new, &row.client_id)
        })
        .await
        .map_err(|e| storage(&e))?;
        self.0.wake();
        Ok(())
    }

    /// Store our edit or delete `post`'s row with no echo (plan E3.3: the
    /// change is stored on its target's row once queued). Returns its
    /// stream seq, the change's place (D22, rule E1).
    ///
    /// # Errors
    /// The storage failed.
    pub async fn queue_change(&self, post: &ChannelPost) -> Result<u64, OutboxError> {
        let payload = serde_json::to_vec(post).map_err(|e| storage(&e))?;
        let row = self.0.new_row(Entry {
            kind: StreamKind::ChannelPost,
            scope: post.scope(),
            client_id: post.message.clone(),
            payload,
            deadline: Deadline::Horizon,
        });
        let (db, owner) = self.0.deps().db()?;
        let seq = db
            .call(move |conn| {
                let new = repo::NewRow {
                    owner_key: &owner,
                    stream: &row.stream,
                    client_id: &row.client_id,
                    kind: row.kind.as_str(),
                    payload: &row.payload,
                    next_at: row.next_at,
                    deadline: row.deadline,
                    depends_on: None,
                };
                repo::enqueue_with_echo(conn, &new, None)
            })
            .await
            .map_err(|e| storage(&e))?;
        self.0.wake();
        u64::try_from(seq).map_err(|e| storage(&e))
    }

    /// Store `post`'s row waiting for its channel's key until `deadline`
    /// (ms), its echo the post already written `awaiting_key`.
    ///
    /// # Errors
    /// The storage failed, or no waiting post is `post.message`.
    pub async fn queue_waiting(
        &self,
        post: &ChannelPost,
        deadline: i64,
    ) -> Result<(), OutboxError> {
        let payload = serde_json::to_vec(post).map_err(|e| storage(&e))?;
        let row = self.0.new_row(Entry {
            kind: StreamKind::ChannelPost,
            scope: post.scope(),
            client_id: post.message.clone(),
            payload,
            deadline: Deadline::At(deadline),
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
                depends_on: None,
            };
            repo::enqueue_waiting_for_message(conn, &new, &row.client_id)
        })
        .await
        .map_err(|e| storage(&e))?;
        // The engine's pass learns the new deadline.
        self.0.wake();
        Ok(())
    }

    /// The posts waiting for their channel's key, of `community` only when
    /// given, each channel's first-composed first.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn waiting(&self, community: Option<&str>) -> Result<Vec<ChannelPost>, OutboxError> {
        let (db, owner) = self.0.deps().db()?;
        let rows = db
            .call(move |conn| repo::waiting_for_key(conn, &owner))
            .await
            .map_err(|e| storage(&e))?;
        Ok(rows
            .into_iter()
            .filter_map(|row| post_of(&super::row_of(row).ok()?))
            .filter(|post| community.is_none_or(|c| post.community == c))
            .collect())
    }

    /// Release the waiting post `message` to its stream: `queued`, due now,
    /// with a write's deadline from now. `false` when it was not waiting.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn release(&self, message: &str) -> Result<bool, OutboxError> {
        let now = self.0.deps().clock.now_ms_i64();
        let deadline =
            now.saturating_add(i64::try_from(OUTBOX_DEADLINE.as_millis()).unwrap_or(i64::MAX));
        let (db, owner) = self.0.deps().db()?;
        let message = message.to_owned();
        let changed = db
            .call(move |conn| repo::release_waiting(conn, &owner, &message, (now, deadline)))
            .await
            .map_err(|e| storage(&e))?;
        self.0.wake();
        Ok(changed)
    }

    /// We left `community`, or were removed from it: its posts still
    /// waiting for the key are discarded (as `mek_transfer`'s
    /// [`super::mek_transfer::DEPARTED`]). Returns them.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_community(
        &self,
        community: &str,
    ) -> Result<Vec<ChannelPost>, OutboxError> {
        let mut discarded = Vec::new();
        for post in self.waiting(Some(community)).await? {
            if self.0.discard(&post.message).await? {
                discarded.push(post);
            }
        }
        Ok(discarded)
    }

    /// Every state change of the outbox from now on.
    #[must_use]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<rekindle_outbox::StateChange> {
        self.0.deps().subscribe()
    }

    /// The post of the row `client_id`, while it is on the outbox.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn post(&self, client_id: &str) -> Result<Option<ChannelPost>, OutboxError> {
        use rekindle_outbox::OutboxDeps as _;
        Ok(self
            .0
            .deps()
            .row(client_id)
            .await?
            .as_ref()
            .and_then(post_of))
    }

    /// Every post of ours still unsettled in `community`'s `channel` will
    /// not reach its readers (our removal, plan E3.2 S9 on the outbox,
    /// E3.3): each settles `failed` with `reason`. Returns them.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn fail_channel(
        &self,
        community: &str,
        channel: &str,
        reason: &str,
    ) -> Result<Vec<ChannelPost>, OutboxError> {
        let rows = (self.0)
            .settle_stream(
                StreamKind::ChannelPost,
                &format!("{community}:{channel}"),
                Err(reason.to_owned()),
            )
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
    match post_of_row(row) {
        Ok(post) => Some(post),
        Err(error) => {
            tracing::warn!(%error, client_id = %row.client_id, "channel post not settled");
            None
        }
    }
}

impl OutboxSlot {
    /// The running outbox's `channel_post` stream.
    pub(crate) fn channel_posts(&self) -> Option<ChannelPosts> {
        self.current.read().clone().map(ChannelPosts)
    }

    /// The lock one release of key-waiting posts holds
    /// ([`OutboxSlot::channel_releases`]).
    pub(crate) fn channel_release_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.channel_releases)
    }
}

impl Host {
    /// The unlocked identity's `channel_post` stream.
    #[must_use]
    pub fn channel_posts(&self) -> Option<ChannelPosts> {
        self.ctx.outbox.channel_posts()
    }
}
