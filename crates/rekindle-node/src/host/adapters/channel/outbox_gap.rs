//! The channel outbox's history-gap marker (plan E3.3, D23 "Gaps": loss is
//! never silent). A departure's head is a head the reader knows exists;
//! when the bounded gap reads (`outbox_read.rs`) still stop below it, the
//! author's entries up to it may be missing. Which channels they were
//! posted in is not known (an entry names its channel only by a tag under
//! its key, and these entries were never read), so the marker is a
//! `messages.history_gap_through` row (E2.5's built pattern,
//! `001_init.sql`) in every channel of the community we read, its sender
//! the author and its seq the head's chain seq. The cursor stays below the
//! head, so a later read can still reach it; once one does, the markers
//! through it are removed, as the history then holds what they stood for.

use rekindle_db::repo::history_gap;
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::ConversationType;

use rekindle_channel::ChannelMessagingDeps as _;

use super::HostChannel;

impl HostChannel {
    /// The channels of `community_id` we read: those whose text scope has
    /// a key we hold.
    fn channels_we_read(&self, community_id: &str) -> Vec<String> {
        let Some(state) = self.governance_state(community_id) else {
            return Vec::new();
        };
        let keys = self.keys();
        let mut channels: Vec<String> = (state.channels.keys())
            .filter(|id| {
                let scope = keys.scope_for_text(community_id, **id);
                keys.current_epoch(community_id, scope).is_some()
            })
            .map(|id| id.to_hex())
            .collect();
        channels.sort_unstable();
        channels
    }

    /// Store `author`'s gap marker through chain seq `through` in every
    /// channel of `community_id` we read (once per channel and seq).
    pub(crate) async fn mark_outbox_gap(
        &self,
        community_id: &str,
        author: &PseudonymKey,
        through: u64,
    ) {
        let channels = self.channels_we_read(community_id);
        let Ok((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (writer, now) = (hex::encode(author.0), self.ctx().clock.now_ms_i64());
        let stored = db
            .call(move |conn| {
                for channel in &channels {
                    let at = (channel.as_str(), ConversationType::Channel);
                    history_gap::insert(conn, &owner, at, &writer, through, now)?;
                }
                Ok(())
            })
            .await;
        if let Err(error) = stored {
            tracing::warn!(community_id, through, %error, "channel outbox gap marker not stored");
        }
    }

    /// Remove `author`'s gap markers through chain seq `reached`: a read
    /// reached that head.
    pub(crate) async fn close_outbox_gap(&self, author: &PseudonymKey, reached: u64) {
        let Ok((owner, db)) = self.owner_and_db() else {
            return;
        };
        let writer = hex::encode(author.0);
        let closed = db
            .call(move |conn| {
                history_gap::close(conn, &owner, ConversationType::Channel, &writer, reached)
            })
            .await;
        if let Err(error) = closed {
            tracing::warn!(reached, %error, "channel outbox gap marker not removed");
        }
    }
}
