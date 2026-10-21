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
