//! `rekindle_outbox::OutboxDeps` over the unlocked identity's database
//! (plan E2.5, D22), for both hosts: the rows in `rekindle_db::repo::
//! outbox`, the attempt through the [`Senders`] the Host installs, the
//! Host's clock and randomness, and each state change on a broadcast the
//! Host's event mapping subscribes to.

mod channel_post;
mod dht_write;
mod dm;
mod media_key;
mod senders;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_db::repo::messages::NewDm;
use rekindle_db::repo::outbox as repo;
use rekindle_db::{Db, DbHandle};
use rekindle_outbox::{
    AttemptOutcome, NewOutboxRow, OutboxDeps, OutboxError, OutboxRow, RowUpdate, StateChange,
    StreamKind,
};
use rekindle_types::message::SendState;
use rekindle_utils::{Clock, RandomSource};
use tokio::sync::broadcast;

pub use channel_post::{ChannelPosts, SUPERSEDED};
pub use dht_write::save_unsent;
pub use dm::OutboxSlot;
pub use media_key::{MediaKeyCarrier, MediaKeyDelivery, MediaKeys, SESSION_ENDED};
pub use senders::{Senders, StreamSender};

/// How many state changes a slow subscriber may fall behind by.
const CHANGES: usize = 256;

/// A direct message's local echo, written with its row; its client id is
/// the row's and it starts `queued`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmEcho {
    pub peer_key: String,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub reply_to: Option<String>,
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
    /// in one transaction (plan audit D-1).
    ///
    /// # Errors
    /// The storage failed; nothing was stored.
    pub(super) async fn mark_removing(
        &self,
        peer: &str,
        unfriended: NewOutboxRow,
    ) -> Result<(), OutboxError> {
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
            };
            rekindle_db::repo::friends::mark_removing(conn, &owner, &peer, &row)
        })
        .await
        .map_err(|e| storage(&e))
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

fn row_of(row: repo::Row) -> Result<OutboxRow, OutboxError> {
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
            let new = repo::NewRow {
                owner_key: &owner,
                stream: &row.stream,
                client_id: &row.client_id,
                kind: row.kind.as_str(),
                payload: &row.payload,
                next_at: row.next_at,
                deadline: row.deadline,
            };
            let dm = echo.as_ref().map(|e| NewDm {
                owner_key: &owner,
                peer_key: &e.peer_key,
                sender_key: &e.sender_key,
                body: &e.body,
                timestamp: e.timestamp,
                is_read: true,
                message_id: &row.client_id,
                reply_to: e.reply_to.as_deref(),
                send_state: SendState::Queued,
                stream_seq: None,
            });
            repo::enqueue_with_echo(conn, &new, dm.as_ref())
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn due(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let rows = db.call(move |c| repo::due(c, &owner, now)).await;
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
        db.call(move |c| repo::next_wake(c, &owner))
            .await
            .map_err(|e| storage(&e))
    }

    async fn row(&self, client_id: &str) -> Result<Option<OutboxRow>, OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        let row = db.call(move |c| repo::get(c, &owner, &id)).await;
        row.map_err(|e| storage(&e))?.map(row_of).transpose()
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

    async fn update(&self, client_id: &str, update: &RowUpdate) -> Result<(), OutboxError> {
        let (db, owner) = self.db()?;
        let (id, update) = (client_id.to_owned(), update.clone());
        db.call(move |c| {
            let change = repo::Update {
                state: update.state,
                attempts: update.attempts,
                next_at: update.next_at,
                deadline: update.deadline,
                wedge_reason: update.wedge_reason.as_deref(),
            };
            repo::update(c, &owner, &id, &change)
        })
        .await
        .map(drop)
        .map_err(|e| storage(&e))
    }

    async fn settle(&self, client_id: &str, state: SendState) -> Result<(), OutboxError> {
        let (db, owner) = self.db()?;
        let id = client_id.to_owned();
        db.call(move |c| repo::settle(c, &owner, &id, state))
            .await
            .map(drop)
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

    fn publish(&self, change: StateChange) {
        // No subscriber yet is not an error: the state is in the database.
        let _ = self.changes.send(change);
    }
}
