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
mod tests {
    use parking_lot::Mutex;
    use rekindle_utils::{OsRandom, SystemClock};

    use super::*;
    use crate::host::adapters::outbox::Senders;

    const OWNER: &str = "aa";

    fn outbox(db: &Arc<rekindle_db::DbHandle>, senders: &Arc<Senders>) -> Outbox<HostOutbox> {
        Outbox::new(HostOutbox::new(
            Arc::clone(db),
            OWNER.into(),
            Arc::new(SystemClock),
            Arc::new(OsRandom),
            Arc::clone(senders),
        ))
    }

    /// A friend list whose writes answer `outcomes` in turn (then
    /// `Landed`), recording the list version each one wrote.
    #[derive(Default)]
    struct FakeList {
        version: Mutex<u32>,
        outcomes: Mutex<Vec<SetOutcome>>,
        wrote: Mutex<Vec<u32>>,
    }

    #[async_trait]
    impl ListWriter for FakeList {
        async fn write(&self) -> Result<SetOutcome, FriendError> {
            self.wrote.lock().push(*self.version.lock());
            let mut outcomes = self.outcomes.lock();
            Ok(if outcomes.is_empty() {
                SetOutcome::Landed
            } else {
                outcomes.remove(0)
            })
        }
    }

    async fn opened() -> (tempfile::TempDir, Arc<rekindle_db::DbHandle>) {
        let dir = tempfile::tempdir().unwrap();
        let db = rekindle_db::open(&dir.path().join("o.db"), &[7; 32]).unwrap();
        db.call(|c| {
            rekindle_db::repo::identity::insert(c, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
        })
        .await
        .unwrap();
        let handle = Arc::new(rekindle_db::DbHandle::default());
        handle.set(db);
        (dir, handle)
    }

    /// Commit a friend-list change with its row, as every change does.
    async fn change(handle: &Arc<rekindle_db::DbHandle>, at: i64) {
        let list = rekindle_db::repo::outbox::ListWrite {
            at,
            due: at,
            deadline: i64::MAX,
        };
        let db = handle.current().unwrap();
        db.call(move |c| rekindle_db::repo::friends::changing_list(c, OWNER, list, |_| Ok(())))
            .await
            .unwrap();
    }

    fn with_list(list: &Arc<FakeList>) -> Arc<Senders> {
        let senders = Arc::new(Senders::default());
        let list: Arc<dyn ListWriter> = Arc::clone(list) as _;
        senders.install(
            StreamKind::DhtWrite,
            Arc::new(DhtWriteSender::new(|_| {}, list)),
        );
        senders
    }

    /// The outbox over `handle`, running (its pump and its appenders).
    fn running(
        handle: &Arc<rekindle_db::DbHandle>,
        senders: &Arc<Senders>,
    ) -> (Arc<Outbox<HostOutbox>>, tokio_util::sync::CancellationToken) {
        let o = Arc::new(outbox(handle, senders));
        let stop = tokio_util::sync::CancellationToken::new();
        let (run, token) = (Arc::clone(&o), stop.clone());
        tokio::spawn(async move { run.run(token).await });
        (o, stop)
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    /// Review RV5 item 5: the change and its row commit together, so a
    /// crash right after the change (no outbox ran) still writes the list
    /// at the next unlock, once, off the pump.
    #[tokio::test]
    async fn a_friend_list_change_lost_to_a_crash_lands_at_the_next_unlock() {
        let (_dir, handle) = opened().await;
        change(&handle, 1).await;
        let list = Arc::new(FakeList::default());
        let (unlocked, stop) = running(&handle, &with_list(&list));
        unlocked.flush(StreamKind::DhtWrite).await.unwrap();
        until("the write", || !list.wrote.lock().is_empty()).await;
        let settled = || async { unlocked.unsettled().await.unwrap() == 0 };
        for _ in 0..200 {
            if settled().await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(unlocked.unsettled().await.unwrap(), 0, "landed, settled");
        assert_eq!(list.wrote.lock().len(), 1);
        stop.cancel();
    }

    /// A write lost to a newer value keeps its row, a later change
    /// replaces it, and the retry writes the latest list.
    #[tokio::test]
    async fn a_superseded_friend_list_write_is_retried_with_the_latest_list() {
        let (_dir, handle) = opened().await;
        let list = Arc::new(FakeList::default());
        list.outcomes.lock().push(SetOutcome::Superseded(
            rekindle_records::port::RecordValue {
                seq: 9,
                data: vec![],
                writer: String::new(),
            },
        ));
        *list.version.lock() = 1;
        let (o, stop) = running(&handle, &with_list(&list));
        change(&handle, 1).await;
        o.wake();
        until("the first write", || list.wrote.lock().len() == 1).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(o.unsettled().await.unwrap(), 1, "kept for its retry");
        *list.version.lock() = 2;
        change(&handle, 2).await;
        assert_eq!(
            o.unsettled().await.unwrap(),
            1,
            "the later change replaced it"
        );
        o.wake();
        until("the retry", || list.wrote.lock().len() == 2).await;
        for _ in 0..200 {
            if o.unsettled().await.unwrap() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(o.unsettled().await.unwrap(), 0);
        assert_eq!(
            *list.wrote.lock(),
            [1, 2],
            "the retry wrote the latest list"
        );
        stop.cancel();
    }

    #[test]
    fn a_payload_round_trips() {
        let write = UnsentWrite {
            record_key: "VLD0:key".into(),
            subkey: 3,
            data: vec![1, 2, 3],
        };
        let back = decode(&encode(&write).unwrap()).unwrap();
        assert_eq!(
            (back.record_key, back.subkey, back.data),
            (write.record_key, write.subkey, write.data)
        );
        assert!(decode(&[0, 0, 0, 1, 0, 9, b'k']).is_none());
    }

    /// What a lock saves survives it, and the next unlock hands it to the
    /// pool once.
    #[tokio::test]
    async fn an_unsent_write_survives_lock_and_unlock_and_lands_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = rekindle_db::open(&dir.path().join("o.db"), &[7; 32]).unwrap();
        db.call(|c| {
            rekindle_db::repo::identity::insert(c, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
        })
        .await
        .unwrap();
        let handle = Arc::new(rekindle_db::DbHandle::default());
        handle.set(db);
        let write = UnsentWrite {
            record_key: "VLD0:key".into(),
            subkey: 3,
            data: vec![1, 2, 3],
        };
        // Lock: saved through an outbox with no sender.
        let locked = outbox(&handle, &Arc::default());
        assert_eq!(enqueue_unsent(&locked, vec![write]).await, 1);
        drop(locked);
        // Unlock: a new outbox over the same rows, the sender installed.
        let senders = Arc::new(Senders::default());
        let held = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&held);
        senders.install(
            StreamKind::DhtWrite,
            Arc::new(DhtWriteSender::new(
                move |w| into.lock().push(w),
                Arc::new(FakeList::default()),
            )),
        );
        let unlocked = outbox(&handle, &senders);
        assert_eq!(unlocked.unsettled().await.unwrap(), 1);
        assert_eq!(unlocked.flush(StreamKind::DhtWrite).await.unwrap(), 1);
        assert_eq!(unlocked.flush(StreamKind::DhtWrite).await.unwrap(), 0);
        assert_eq!(unlocked.unsettled().await.unwrap(), 0);
        let held = held.lock();
        assert_eq!(held.len(), 1, "landed once");
        assert_eq!(
            (held[0].subkey, held[0].data.as_slice()),
            (3, &[1, 2, 3][..])
        );
    }
}
