use std::sync::Arc;

use async_trait::async_trait;
use rekindle_db::repo::messages::own_dm;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxDeps as _, OutboxRow, StreamKind,
};
use rekindle_types::message::SendState;
use rekindle_utils::{OsRandom, SystemClock};

use super::{DmEcho, HostOutbox, Senders, StreamSender};

struct AlwaysSent;

#[async_trait]
impl StreamSender for AlwaysSent {
    async fn send(&self, _: &OutboxRow) -> AttemptOutcome {
        AttemptOutcome::Sent
    }
}

const OWNER: &str = "aa";

async fn host(dir: &std::path::Path, senders: Arc<Senders>) -> HostOutbox {
    let db = rekindle_db::open(&dir.join("o.db"), &[7; 32]).unwrap();
    db.call(|c| {
        rekindle_db::repo::identity::insert(c, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
    })
    .await
    .unwrap();
    let handle = Arc::new(rekindle_db::DbHandle::default());
    handle.set(db);
    HostOutbox::new(
        handle,
        OWNER.into(),
        Arc::new(SystemClock),
        Arc::new(OsRandom),
        senders,
    )
}

#[tokio::test]
async fn a_row_waits_for_its_sender_and_projects_onto_its_echo() {
    let dir = tempfile::tempdir().unwrap();
    let senders = Arc::new(Senders::default());
    let outbox = Outbox::new(host(dir.path(), senders.clone()).await);
    let mut changes = outbox.deps().subscribe();
    let entry = Entry {
        kind: StreamKind::Dm,
        scope: "bob".into(),
        client_id: "c1".into(),
        payload: b"frame".to_vec(),
        deadline: Deadline::Horizon,
    };
    let echo = DmEcho {
        peer_key: "bob".into(),
        sender_key: OWNER.into(),
        body: "hi".into(),
        timestamp: 1,
        reply_to: None,
    };
    outbox.enqueue(entry, Some(echo)).await.unwrap();
    assert_eq!(changes.recv().await.unwrap().state, SendState::Queued);
    // No sender: the row is not attempted and stays queued.
    outbox.pump().await.unwrap();
    let row = outbox.deps().row("c1").await.unwrap().unwrap();
    assert_eq!((row.state, row.attempts), (SendState::Queued, 1));
    senders.install(StreamKind::Dm, Arc::new(AlwaysSent));
    outbox.retry("c1").await.unwrap();
    outbox.pump().await.unwrap();
    assert_eq!(changes.recv().await.unwrap().state, SendState::Sent);
    let db = outbox.deps().db.current().unwrap();
    let echoed = db.call(|c| own_dm(c, OWNER, "c1")).await.unwrap().unwrap();
    assert_eq!(echoed.row.send_state, SendState::Sent);
    assert_eq!(outbox.unsettled().await.unwrap(), 1);
}

/// Records the client id of every attempt.
#[derive(Default)]
struct Recording(parking_lot::Mutex<Vec<String>>);

#[async_trait]
impl StreamSender for Recording {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        self.0.lock().push(row.client_id.clone());
        AttemptOutcome::Sent
    }
}

fn restarted(db: Arc<rekindle_db::DbHandle>, senders: Arc<Senders>) -> Outbox<HostOutbox> {
    Outbox::new(HostOutbox::new(
        db,
        OWNER.into(),
        Arc::new(SystemClock),
        Arc::new(OsRandom),
        senders,
    ))
}

/// D22: killed right after Send, the message goes out on the next start
/// under the same client id, from the stored row alone.
#[tokio::test]
async fn a_row_committed_before_a_kill_is_sent_after_the_restart_under_its_client_id() {
    let dir = tempfile::tempdir().unwrap();
    let senders = Arc::new(Senders::default());
    let first = Outbox::new(host(dir.path(), senders.clone()).await);
    let db = Arc::clone(&first.deps().db);
    let entry = Entry {
        kind: StreamKind::Dm,
        scope: "bob".into(),
        client_id: "c-kill".into(),
        payload: b"frame".to_vec(),
        deadline: Deadline::Horizon,
    };
    first.enqueue(entry, None).await.unwrap();
    drop(first);

    let recording = Arc::new(Recording::default());
    senders.install(StreamKind::Dm, recording.clone());
    let second = restarted(db, senders);
    second.pump().await.unwrap();
    assert_eq!(*recording.0.lock(), ["c-kill"]);
    let row = second.deps().row("c-kill").await.unwrap().unwrap();
    assert_eq!(
        row.state,
        SendState::Sent,
        "sent, waiting for the peer's DmAck"
    );
}

/// A re-delivered entry is applied once, across a restart too.
#[tokio::test]
async fn a_redelivered_entry_is_received_once() {
    let dir = tempfile::tempdir().unwrap();
    let senders = Arc::new(Senders::default());
    let first = Outbox::new(host(dir.path(), senders.clone()).await);
    assert!(first.receive("dm:bob", "c9").await.unwrap());
    assert!(!first.receive("dm:bob", "c9").await.unwrap());
    let second = restarted(Arc::clone(&first.deps().db), senders);
    assert!(!second.receive("dm:bob", "c9").await.unwrap());
    assert!(
        second.receive("dm:carol", "c9").await.unwrap(),
        "per stream"
    );
}
