//! Outbox entries read before their channel merged (plan E3.3, D23 "Cursor
//! and authors"; live finding CH1). An author's post in a channel it has
//! just created can reach a reader's diff before that channel's
//! `ChannelCreated` op merges there: the tag maps to no channel of the
//! merged state while the reader holds the entry's key, so the entry
//! passes (`outbox_read.rs`) and the cursor moves past it. The writer is
//! noted here, and when a merge creates a channel the noted writers'
//! cursors are forgotten and their outboxes diffed again, the way an older
//! community key re-reads every outbox (plan D16, `catchup.rs`); an entry
//! already applied comes back as a duplicate its ingest drops
//! (`stream_cursors::reset_stream`).
//!
//! The entry still passes rather than holding the cursor
//! ([`rekindle_sync::stream::Held::Pending`]): a thread reply's tag maps to
//! no channel by design, and a garbage tag never will, so a hold would wedge
//! the author's outbox behind them for good. What is kept is one row per
//! writer (author at its member record) per community, however many
//! entries of it passed, so a writer of garbage tags grows nothing past its
//! one row; the set is bounded by the writers a catch-up reads (the
//! community's members and departed authors, `catch_up_outboxes`), and each
//! created channel empties it.

use rekindle_types::id::PseudonymKey;

use super::outbox_read::outbox_stream;
use super::HostChannel;

impl HostChannel {
    /// Note that an entry of `author`'s outbox at `member_record` mapped
    /// to no channel of `community_id`'s merged state.
    pub(super) fn note_unmapped(&self, community_id: &str, author: &[u8; 32], member_record: &str) {
        let map = self.ctx().communities.map().read();
        if let Some(community) = map.get(community_id) {
            (community.outbox_unmapped.lock()).insert((*author, member_record.to_owned()));
        }
    }

    /// A merge created a channel in `community_id`: forget the cursor of
    /// every writer noted since the last creation and diff its outbox
    /// again, so its entries of the new channel are read. Returns the
    /// writers re-read.
    pub(crate) async fn reread_unmapped(&self, community_id: &str) -> usize {
        let writers = {
            let map = self.ctx().communities.map().read();
            (map.get(community_id)).map(|c| std::mem::take(&mut *c.outbox_unmapped.lock()))
        };
        let Some(writers) = writers.filter(|w| !w.is_empty()) else {
            return 0;
        };
        let (Ok(owner), Ok(db)) = (self.ctx().require_owner(), self.ctx().db.current()) else {
            return 0;
        };
        let stream = outbox_stream(community_id);
        for (author, member_record) in &writers {
            let (owner, stream) = (owner.clone(), stream.clone());
            let writer = format!("{}:{member_record}", hex::encode(author));
            let reset = db.call(move |conn| {
                rekindle_db::repo::stream_cursors::reset(conn, &owner, &stream, &writer)
            });
            if let Err(error) = reset.await {
                tracing::warn!(community_id, %error, "outbox cursor not reset for a new channel");
                continue;
            }
            let author = PseudonymKey(*author);
            (self.read_author_outbox(community_id, &author, member_record, false)).await;
        }
        tracing::info!(
            community_id,
            writers = writers.len(),
            "channel outboxes re-read for a new channel"
        );
        writers.len()
    }
}
