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
