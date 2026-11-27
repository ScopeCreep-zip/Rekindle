//! `rekindle_outbox::OutboxDeps` over the unlocked identity's database
//! (plan E2.5, D22), for both hosts: the rows in `rekindle_db::repo::
//! outbox`, the attempt through the [`Senders`] the Host installs, the
//! Host's clock and randomness, and each state change on a broadcast the
//! Host's event mapping subscribes to.

mod batch;
mod bootstrap;
#[cfg(test)]
mod bootstrap_tests;
mod channel_post;
pub(crate) mod dht_write;
mod dm;
mod forum_starters;
mod governance_op;
mod governance_publication;
mod media_key;
pub(crate) mod mek_transfer;
mod senders;
mod stored;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_db::repo::messages::NewDm;
use rekindle_db::repo::outbox as repo;
use rekindle_db::{Db, DbHandle};
use rekindle_outbox::state::{apply, Moved, Step, StoredSend, Write};
use rekindle_outbox::{
    AttemptOutcome, Landed, NewOutboxRow, OutboxDeps, OutboxError, OutboxRow, PeerAck, RowUpdate,
    StateChange, StreamKind,
};
use rekindle_types::message::SendState;
use rekindle_utils::{Clock, RandomSource};
use tokio::sync::broadcast;

use stored::stored_of;

pub use channel_post::{post_of_row, ChannelPosts};
pub use dht_write::save_unsent;
pub use dm::OutboxSlot;
pub use governance_op::{GovernanceOps, Published};
pub use media_key::{MediaKeyCarrier, MediaKeyDelivery, MediaKeyStreams, SESSION_ENDED};
pub use mek_transfer::MekTransfers;
pub use senders::{Senders, StreamSender};

/// How many state changes a slow subscriber may fall behind by.
const CHANGES: usize = 256;

/// A direct message's local echo, written with its row; its client id is
/// the row's and it starts `queued`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmEcho {
    /// The conversation: the peer, or the group's id (plan E2.6).
    pub peer_key: String,
    pub conversation_type: rekindle_types::message::ConversationType,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub reply_to: Option<String>,
    /// The timer the message carries (plan E7.1); the outbox's append
    /// sets our copy's `expire_at` from it.
    pub expire_seconds: u32,
}

/// The outbox's host over one unlocked identity.
pub struct HostOutbox {
    db: Arc<DbHandle>,
    owner_key: String,
    clock: Arc<dyn Clock>,
    rng: Arc<dyn RandomSource>,
    senders: Arc<Senders>,
    changes: broadcast::Sender<StateChange>,
}

impl HostOutbox {
    #[must_use]
    pub fn new(
        db: Arc<DbHandle>,
        owner_key: String,
        clock: Arc<dyn Clock>,
        rng: Arc<dyn RandomSource>,
        senders: Arc<Senders>,
    ) -> Self {
        Self {
            db,
            owner_key,
            clock,
            rng,
            senders,
            changes: broadcast::channel(CHANGES).0,
        }
    }

    /// Every state change from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.changes.subscribe()
    }

    /// Mark `peer` removing and store `unfriended`, its `Unfriended` row,
    /// and the friend list's row (`list`), in one transaction (plan audit
    /// D-1, review RV5 item 5).
    ///
    /// # Errors
    /// The storage failed; nothing was stored.
    pub(super) async fn mark_removing(
        &self,
        peer: &str,
        unfriended: NewOutboxRow,
        list: repo::ListWrite,
    ) -> Result<Vec<repo::Row>, OutboxError> {
        let (db, owner) = self.db()?;
        let peer = peer.to_owned();
        db.call(move |conn| {
            let row = repo::NewRow {
                owner_key: &owner,
                stream: &unfriended.stream,
                client_id: &unfriended.client_id,
                kind: unfriended.kind.as_str(),
                payload: &unfriended.payload,
                next_at: unfriended.next_at,
                deadline: unfriended.deadline,
                depends_on: None,
            };
            rekindle_db::repo::friends::changing_list(conn, &owner, list, |conn| {
                rekindle_db::repo::friends::mark_removing(conn, &owner, &peer, &row)
            })
        })
        .await
        .map_err(|e| storage(&e))
    }

    /// Store `row` unless a row of ours already holds its client id
    /// ([`repo::enqueue_once`]) and publish it as `queued`; `false` when
    /// it was already queued. The caller wakes the engine.
    ///
    /// # Errors
    /// The storage failed; nothing was stored.
    pub(super) async fn insert_once(&self, row: NewOutboxRow) -> Result<bool, OutboxError> {
        let (db, owner) = self.db()?;
        let stored = row.clone();
        let seq = db
            .call(move |conn| {
                let dep =
                    repo::latest_with_prefix(conn, &owner, &stored.stream, &stored.waits_for)?;
                let seq = repo::enqueue_once(
                    conn,
                    &repo::NewRow {
                        owner_key: &owner,
                        stream: &stored.stream,
                        client_id: &stored.client_id,
                        kind: stored.kind.as_str(),
                        payload: &stored.payload,
                        next_at: stored.next_at,
                        deadline: stored.deadline,
                        depends_on: dep.as_deref(),
                    },
                )?;
                if seq.is_some() {
                    let (stream, id) = (&stored.stream, &stored.client_id);
                    repo::take_over_dependents(conn, &owner, stream, &stored.takes_over, id)?;
                }
                Ok(seq)
            })
            .await
            .map_err(|e| storage(&e))?;
        let Some(stream_seq) = seq else {
            return Ok(false);
        };
        let _ = self.changes.send(StateChange {
            stream: row.stream,
            stream_seq,
            client_id: row.client_id,
            kind: row.kind,
            state: SendState::Queued,
            wedge_reason: None,
            retry: None,
        });
        Ok(true)
    }

    /// The open database and the owner its rows belong to.
    fn db(&self) -> Result<(Db, String), OutboxError> {
        let db = self.db.current().map_err(|e| storage(&e))?;
        Ok((db, self.owner_key.clone()))
    }
}

fn storage(e: &impl std::fmt::Display) -> OutboxError {
    OutboxError::Storage(e.to_string())
}

pub(super) fn row_of(row: repo::Row) -> Result<OutboxRow, OutboxError> {
    let kind = StreamKind::parse(&row.kind)
        .ok_or_else(|| OutboxError::Storage(format!("unknown stream kind {}", row.kind)))?;
    Ok(OutboxRow {
        stream: row.stream,
        stream_seq: row.stream_seq,
        client_id: row.client_id,
        kind,
        payload: row.payload,
        state: row.state,
        attempts: row.attempts,
        next_at: row.next_at,
        deadline: row.deadline,
        wedge_reason: row.wedge_reason,
        sent_at: row.sent_at,
        depends_on: row.depends_on,
    })
}

fn rows_of(rows: Vec<repo::Row>) -> Result<Vec<OutboxRow>, OutboxError> {
    rows.into_iter().map(row_of).collect()
}

#[async_trait]
impl OutboxDeps for HostOutbox {
    type Echo = Option<DmEcho>;

    fn clock(&self) -> &dyn Clock {
        &*self.clock
    }

    fn rng(&self) -> &dyn RandomSource {
        &*self.rng
    }

    async fn insert(&self, row: NewOutboxRow, echo: Option<DmEcho>) -> Result<i64, OutboxError> {
        let (db, owner) = self.db()?;
        db.call(move |conn| {
            let dep = repo::latest_with_prefix(conn, &owner, &row.stream, &row.waits_for)?;
            let new = repo::NewRow {
                owner_key: &owner,
                stream: &row.stream,
                client_id: &row.client_id,
                kind: row.kind.as_str(),
                payload: &row.payload,
                next_at: row.next_at,
                deadline: row.deadline,
                depends_on: dep.as_deref(),
            };
            let dm = echo.as_ref().map(|e| NewDm {
                owner_key: &owner,
                peer_key: &e.peer_key,
                conversation_type: e.conversation_type,
                expire_seconds: e.expire_seconds,
                expire_at: None,
                sender_key: &e.sender_key,
                body: &e.body,
                timestamp: e.timestamp,
                is_read: true,
                message_id: &row.client_id,
                reply_to: e.reply_to.as_deref(),
                send_state: SendState::Queued,
                stream_seq: None,
            });
            let seq = repo::enqueue_with_echo(conn, &new, dm.as_ref())?;
            repo::take_over_dependents(conn, &owner, &row.stream, &row.takes_over, &row.client_id)?;
            Ok(seq)
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn due(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let kinds: Vec<&str> = StreamKind::by_dependency().collect();
        let rows = db.call(move |c| repo::due(c, &owner, now, &kinds)).await;
        rows_of(rows.map_err(|e| storage(&e))?)
    }

    async fn stream_rows(&self, stream: &str) -> Result<Vec<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let stream = stream.to_owned();
        let rows = db
            .call(move |c| repo::stream_rows(c, &owner, &stream))
            .await;
        rows_of(rows.map_err(|e| storage(&e))?)
    }

    async fn prefix_rows(&self, prefix: &str) -> Result<Vec<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let prefix = prefix.to_owned();
        let rows = db
            .call(move |c| repo::prefix_rows(c, &owner, &prefix))
            .await;
        rows_of(rows.map_err(|e| storage(&e))?)
    }

    async fn expired(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let rows = db.call(move |c| repo::expired(c, &owner, now)).await;
        rows_of(rows.map_err(|e| storage(&e))?)
    }

    async fn next_wake(&self) -> Result<Option<i64>, OutboxError> {
        let (db, owner) = self.db()?;
        let kinds: Vec<&str> = StreamKind::by_dependency().collect();
        db.call(move |c| repo::next_wake(c, &owner, &kinds))
            .await
            .map_err(|e| storage(&e))
    }

    async fn row(&self, client_id: &str) -> Result<Option<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        let row = db.call(move |c| repo::get(c, &owner, &id)).await;
        row.map_err(|e| storage(&e))?.map(row_of).transpose()
    }

    async fn reschedule_sent(&self, stream: &str, at: i64) -> Result<usize, OutboxError> {
        let (db, owner) = self.db()?;
        let stream = stream.to_owned();
        db.call(move |c| repo::reschedule_sent(c, &owner, &stream, at))
            .await
            .map_err(|e| storage(&e))
    }

    async fn row_at(
        &self,
        stream: &str,
        stream_seq: i64,
    ) -> Result<Option<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let stream = stream.to_owned();
        let row = db
            .call(move |c| repo::get_by_seq(c, &owner, &stream, stream_seq))
            .await;
        row.map_err(|e| storage(&e))?.map(row_of).transpose()
    }

    async fn update(
        &self,
        client_id: &str,
        update: &RowUpdate,
    ) -> Result<Option<Moved>, OutboxError> {
        let (db, owner) = self.db()?;
        let (id, update) = (client_id.to_owned(), update.clone());
        let step = if update.retry {
            Step::Retry
        } else {
            Step::Update
        };
        // One storage call reads the row as it is now and writes it: the
        // state merges against that, never the row an attempt began from.
        let changes = self.changes.clone();
        db.call(move |c| {
            let Some((write, moved)) = repo::stored_state(c, &owner, &id)?
                .map(|row| apply(stored_of(row), update.state, step))
            else {
                return Ok(None);
            };
            let tx = c.unchecked_transaction()?;
            let abandoned = match write {
                Write::Settle(state) => forum_starters::settle(&tx, &owner, &id, state)?,
                Write::Store(state) => {
                    let change = repo::Update {
                        state,
                        attempts: update.attempts,
                        next_at: update.next_at,
                        deadline: update.deadline,
                        wedge_reason: update.wedge_reason.as_deref(),
                    };
                    repo::update_in(&tx, &owner, &id, &change)?;
                    forum_starters::on_update(&tx, &owner, &id, state)?
                }
            };
            tx.commit()?;
            forum_starters::publish_abandoned(&changes, abandoned, forum_starters::CREATION_FAILED);
            Ok(Some(moved))
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn note_ack(&self, client_id: &str, ack: PeerAck) -> Result<Option<Moved>, OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        let stored = match ack {
            PeerAck::Delivered => SendState::Delivered,
            PeerAck::Read => SendState::Read,
        };
        db.call(move |c| {
            let Some(before) = repo::stored_state(c, &owner, &id)?.map(stored_of) else {
                return Ok(None);
            };
            if !repo::note_ack(c, &owner, &id, stored)? {
                return Ok(None);
            }
            let after = StoredSend {
                state: before.state,
                ack: before.ack.max(Some(ack)),
            };
            Ok(Some(Moved {
                before: before.shown(),
                after: after.shown(),
            }))
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn reschedule_queued(
        &self,
        client_id: &str,
        attempts: u32,
        next_at: i64,
    ) -> Result<(), OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        db.call(move |c| repo::reschedule_queued(c, &owner, &id, attempts, next_at))
            .await
            .map(drop)
            .map_err(|e| storage(&e))
    }

    async fn land(
        &self,
        client_id: &str,
        attempts: u32,
        next_at: i64,
    ) -> Result<Landed, OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        let landing = db
            .call(move |c| repo::land(c, &owner, &id, attempts, next_at))
            .await
            .map_err(|e| storage(&e))?;
        Ok(match landing {
            repo::Landing::Unchanged => Landed::Unchanged,
            repo::Landing::Waiting => Landed::Waiting,
            repo::Landing::Settled(state) => Landed::Settled(state),
        })
    }

    /// A `dm:<peer>` stream's entries are noted under `<peer>`, a
    /// `group_dm:<id>` stream's under its stream name (`chain_entries`).
    async fn on_chain(&self, stream: &str, seqs: &[i64]) -> Result<Vec<i64>, OutboxError> {
        let chain = match stream.strip_prefix("dm:") {
            Some(peer) => peer.to_owned(),
            None if stream.starts_with("group_dm:") => stream.to_owned(),
            None => return Ok(Vec::new()),
        };
        let (db, owner) = self.db()?;
        let seqs = seqs.to_vec();
        db.call(move |c| {
            let mut on = Vec::new();
            for seq in seqs {
                if rekindle_db::repo::chain_entries::has_landed(c, &owner, &chain, seq)? {
                    on.push(seq);
                }
            }
            Ok(on)
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn settle(
        &self,
        client_id: &str,
        state: SendState,
    ) -> Result<Option<Moved>, OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        let changes = self.changes.clone();
        db.call(move |c| {
            let Some((write, moved)) = repo::stored_state(c, &owner, &id)?
                .map(|row| apply(stored_of(row), state, Step::Settle))
            else {
                return Ok(None);
            };
            let (Write::Settle(state) | Write::Store(state)) = write;
            let tx = c.unchecked_transaction()?;
            let abandoned = forum_starters::settle(&tx, &owner, &id, state)?;
            tx.commit()?;
            forum_starters::publish_abandoned(&changes, abandoned, forum_starters::CREATION_FAILED);
            Ok(Some(moved))
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn read_through(
        &self,
        stream: &str,
        scope: &str,
        up_to: i64,
    ) -> Result<Vec<(i64, String)>, OutboxError> {
        let (db, owner) = self.db()?;
        let (stream, peer) = (stream.to_owned(), scope.to_owned());
        db.call(move |c| repo::read_through(c, &owner, &stream, &peer, up_to))
            .await
            .map_err(|e| storage(&e))
    }

    async fn deliver(
        &self,
        stream: &str,
        scope: &str,
        cover: rekindle_outbox::Cover,
    ) -> Result<Vec<(i64, String)>, OutboxError> {
        let (db, owner) = self.db()?;
        let (stream, peer) = (stream.to_owned(), scope.to_owned());
        db.call(move |c| match cover {
            rekindle_outbox::Cover::Seq(seq) => repo::deliver_seq(c, &owner, &stream, &peer, seq),
            rekindle_outbox::Cover::ChainThrough(t) => {
                repo::deliver_chain_through(c, &owner, &stream, &peer, t)
            }
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn mark_received(&self, stream: &str, client_id: &str) -> Result<bool, OutboxError> {
        let (db, owner) = self.db()?;
        let (stream, id) = (stream.to_owned(), client_id.to_owned());
        db.call(move |c| repo::mark_received(c, &owner, &stream, &id))
            .await
            .map_err(|e| storage(&e))
    }

    async fn unsettled_count(&self) -> Result<u64, OutboxError> {
        let (db, owner) = self.db()?;
        let count = db.call(move |c| repo::unsettled_count(c, &owner)).await;
        u64::try_from(count.map_err(|e| storage(&e))?).map_err(|e| storage(&e))
    }

    async fn attempt(&self, row: &OutboxRow) -> AttemptOutcome {
        self.senders.send(row).await
    }

    async fn attempt_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        self.senders.send_stream(rows).await
    }

    async fn append(
        &self,
        kind: StreamKind,
        stream: &str,
        frames: &[(Vec<u8>, i64)],
    ) -> Vec<AttemptOutcome> {
        self.senders.append(kind, stream, frames).await
    }

    async fn call(&self, kind: StreamKind, stream: &str, frame: Vec<u8>) -> AttemptOutcome {
        self.senders.call(kind, stream, frame).await
    }

    fn publish(&self, change: StateChange) {
        // No subscriber yet is not an error: the state is in the database.
        let _ = self.changes.send(change);
    }
}
