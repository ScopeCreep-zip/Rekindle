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

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxDeps as _, OutboxRow, StreamKind,
};
use rekindle_records::port::{RecordPort, UnsentWrite};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::daemon::dispatch::DaemonContext;

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
    /// unsent: install the `dht_write` sender and flush its streams now.
    pub(crate) async fn restore_unsent(&self, pool: Arc<dyn RecordPort>) {
        let sender = DhtWriteSender::new(move |write| pool.restore_held(vec![write]));
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

/// The `dht_write` stream's sender: hands the write to the pool's held
/// set, which owns it reaching consensus; the row then settles.
struct DhtWriteSender {
    hold: Box<dyn Fn(UnsentWrite) + Send + Sync>,
}

impl DhtWriteSender {
    fn new(hold: impl Fn(UnsentWrite) + Send + Sync + 'static) -> Self {
        Self {
            hold: Box::new(hold),
        }
    }
}

#[async_trait]
impl StreamSender for DhtWriteSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        match decode(&row.payload) {
            Some(write) => {
                (self.hold)(write);
                AttemptOutcome::Sent
            }
            None => AttemptOutcome::Wedged("unreadable dht_write payload".to_owned()),
        }
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
            Arc::new(DhtWriteSender::new(move |w| into.lock().push(w))),
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
