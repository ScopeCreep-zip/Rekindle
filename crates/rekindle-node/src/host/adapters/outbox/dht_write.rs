//! The `dht_write` stream (plan E2.5, D22; absorbs C7.6h's unsent-write
//! table):
//! the own-record writes a lock left unsent are the user's send intents,
//! stored as outbox rows, one stream per record slot.
//!
//! Order: at lock, after the pool drained, [`save_unsent`] enqueues what
//! the pool still held; the rows persist with the rest of the outbox. At
//! unlock, [`OutboxSlot::restore_unsent`] runs right after the record pool
//! starts and before the session writes anything: it installs the stream's
//! sender and flushes the stream at once, so each row is handed to the
//! pool once and settles, and the session's own newer durable writes
//! replace them slot by slot. The pool's held set then owns the value
//! reaching consensus (and a write still held at the next lock is saved
//! again).
//!
//! The friend list is the stream's one row written with the change, in its
//! transaction, not at lock (review RV5 item 5,
//! `rekindle_db::repo::friends::changing_list`): it replaces the row not
//! landed yet, and its attempt writes the list built from local state then,
//! so a later change supersedes an earlier one and a retry writes the
//! latest list. The write is handed off the pump like a chain append
//! ([`AttemptOutcome::Appending`]): the row stays `queued` while it is in
//! flight and settles on its landing, so DM hints never wait behind a DHT
//! write. A lock or crash leaves it for the next unlock, whose flush
//! resumes it.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_db::repo::outbox::FRIEND_LIST_STREAM;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxDeps as _, OutboxRow, StreamKind,
};
use rekindle_records::port::{RecordPort, SetOutcome, UnsentWrite};
use rekindle_transport::operations::friend::{self, FriendError};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::friends::HostFriends;

/// Writes our friend list from local state (the transport's builder).
#[async_trait]
pub(crate) trait ListWriter: Send + Sync {
    async fn write(&self) -> Result<SetOutcome, FriendError>;
}

/// The session's friend list, written through its record pool.
pub(crate) struct HostListWriter(pub(crate) HostFriends);

#[async_trait]
impl ListWriter for HostListWriter {
    async fn write(&self) -> Result<SetOutcome, FriendError> {
        friend::write_friend_list(&self.0).await
    }
}

/// The friend-list row's attempt: it settles only once the record holds
/// the list. A write below consensus, offline or lost to a newer value, or
/// one that could not be made (no record or pool yet), is attempted again
/// with the list as it is then, until the row's deadline.
fn landed(outcome: Result<SetOutcome, FriendError>) -> AttemptOutcome {
    match outcome {
        Ok(SetOutcome::Landed | SetOutcome::Unchanged) => AttemptOutcome::Sent,
        Ok(missed) => {
            tracing::debug!(?missed, "friend list not stored yet; attempted again");
            AttemptOutcome::Deferred
        }
        Err(error) => {
            tracing::warn!(%error, "friend list not written; attempted again");
            AttemptOutcome::Deferred
        }
    }
}

/// The stored payload: `subkey (u32 BE) ‖ key length (u16 BE) ‖ key ‖ data`.
fn encode(write: &UnsentWrite) -> Option<Vec<u8>> {
    let key = write.record_key.as_bytes();
    let len = u16::try_from(key.len()).ok()?;
    let mut out = Vec::with_capacity(6 + key.len() + write.data.len());
    out.extend_from_slice(&write.subkey.to_be_bytes());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(&write.data);
    Some(out)
}

fn decode(payload: &[u8]) -> Option<UnsentWrite> {
    let subkey = u32::from_be_bytes(payload.get(..4)?.try_into().ok()?);
    let len = usize::from(u16::from_be_bytes(payload.get(4..6)?.try_into().ok()?));
    let record_key = std::str::from_utf8(payload.get(6..6 + len)?)
        .ok()?
        .to_owned();
    Some(UnsentWrite {
        record_key,
        subkey,
        data: payload.get(6 + len..)?.to_vec(),
    })
}

/// Enqueue `writes` on `outbox`'s `dht_write` streams; how many were stored.
async fn enqueue_unsent(outbox: &Outbox<HostOutbox>, writes: Vec<UnsentWrite>) -> usize {
    let at = outbox.deps().clock().now_ms_i64();
    let mut saved = 0;
    for write in writes {
        let scope = format!("{}/{}", write.record_key, write.subkey);
        let Some(payload) = encode(&write) else {
            tracing::warn!(record = %write.record_key, "an unsent write's key is too long to store");
            continue;
        };
        let entry = Entry {
            kind: StreamKind::DhtWrite,
            client_id: format!("{scope}@{at}"),
            scope,
            payload,
            deadline: Deadline::Horizon,
        };
        match outbox.enqueue(entry, None).await {
            Ok(_) => saved += 1,
            Err(error) => tracing::warn!(%error, "saving an unsent DHT write failed"),
        }
    }
    saved
}

/// Keep the drained pool's held writes for `owner`'s next unlock as
/// `dht_write` rows; how many were saved. The only save: both hosts call
/// it at lock.
pub async fn save_unsent(ctx: &DaemonContext, owner: &str, writes: Vec<UnsentWrite>) -> usize {
    let outbox = Outbox::new(HostOutbox::new(
        Arc::clone(&ctx.db),
        owner.to_owned(),
        Arc::clone(&ctx.clock),
        Arc::clone(&ctx.rng),
        Arc::default(),
    ));
    enqueue_unsent(&outbox, writes).await
}

impl OutboxSlot {
    /// Hold, on the session's new `pool`, the writes the last lock left
    /// unsent, and resume the friend list's row not landed: install the
    /// `dht_write` sender and flush its streams now.
    pub(crate) async fn restore_unsent(
        &self,
        pool: Arc<dyn RecordPort>,
        list: Arc<dyn ListWriter>,
    ) {
        let sender = DhtWriteSender::new(move |write| pool.restore_held(vec![write]), list);
        self.senders.install(StreamKind::DhtWrite, Arc::new(sender));
        let Some(outbox) = self.current.read().clone() else {
            return;
        };
        match outbox.flush(StreamKind::DhtWrite).await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "restored DHT writes the last lock left unsent"),
            Err(error) => tracing::warn!(%error, "restoring the unsent DHT writes failed"),
        }
    }
}

/// The `dht_write` stream's sender: hands a saved write to the pool's held
/// set, which owns it reaching consensus, and the row then settles; the
/// friend list's row is written by `list` ([`landed`]).
struct DhtWriteSender {
    hold: Box<dyn Fn(UnsentWrite) + Send + Sync>,
    list: Arc<dyn ListWriter>,
}

impl DhtWriteSender {
    fn new(hold: impl Fn(UnsentWrite) + Send + Sync + 'static, list: Arc<dyn ListWriter>) -> Self {
        Self {
            hold: Box::new(hold),
            list,
        }
    }
}

#[async_trait]
impl StreamSender for DhtWriteSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        if row.stream == FRIEND_LIST_STREAM {
            return AttemptOutcome::Appending(Vec::new());
        }
        match decode(&row.payload) {
            Some(write) => {
                (self.hold)(write);
                AttemptOutcome::Sent
            }
            None => AttemptOutcome::Wedged("unreadable dht_write payload".to_owned()),
        }
    }

    /// The friend list's write, handed off by [`Self::send`]: one row is
    /// ever queued on its stream, so one write lands it.
    async fn append(&self, stream: &str, frames: &[(Vec<u8>, i64)]) -> Vec<AttemptOutcome> {
        if stream != FRIEND_LIST_STREAM || frames.is_empty() {
            return frames
                .iter()
                .take(1)
                .map(|_| AttemptOutcome::Wedged(format!("{stream} has no appender")))
                .collect();
        }
        let outcome = landed(self.list.write().await);
        let settled = frames.len().min(if outcome == AttemptOutcome::Sent {
            frames.len()
        } else {
            1
        });
        std::iter::repeat_n(outcome, settled).collect()
    }
}

#[cfg(test)]
#[path = "dht_write_tests.rs"]
mod tests;
