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
        conversation_type: rekindle_types::message::ConversationType::Dm,
        sender_key: OWNER.into(),
        body: "hi".into(),
        timestamp: 1,
        reply_to: None,
        expire_seconds: 0,
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

async fn queued_post(
    outbox: &Arc<Outbox<HostOutbox>>,
    id: &'static str,
) -> rekindle_types::message::ChannelPost {
    let db = outbox.deps().db.current().unwrap();
    db.call(move |c| {
        rekindle_db::test_support::queued_channel_post(c, OWNER, "ch1", id);
        Ok(())
    })
    .await
    .unwrap();
    rekindle_types::message::ChannelPost {
        community: "c1".into(),
        channel: "ch1".into(),
        message: id.into(),
        entry: rekindle_types::message::PostEntry::Message,
    }
}

async fn post_state(outbox: &Arc<Outbox<HostOutbox>>, id: &'static str) -> Option<SendState> {
    let db = outbox.deps().db.current().unwrap();
    (db.call(move |c| rekindle_db::repo::messages::send_state(c, OWNER, id))
        .await)
        .unwrap()
}

/// A post of `community` composed before its key (plan E3.2 S7l, F9-1):
/// its `messages` row `awaiting_key`, then its row waiting until
/// `deadline`.
async fn waiting_post(
    outbox: &Arc<Outbox<HostOutbox>>,
    community: &str,
    id: &'static str,
    deadline: i64,
) -> rekindle_types::message::ChannelPost {
    let db = outbox.deps().db.current().unwrap();
    db.call(move |c| {
        rekindle_db::test_support::waiting_channel_post(c, OWNER, "ch1", id);
        Ok(())
    })
    .await
    .unwrap();
    let post = rekindle_types::message::ChannelPost {
        community: community.into(),
        channel: "ch1".into(),
        message: id.into(),
        entry: rekindle_types::message::PostEntry::Message,
    };
    let posts = super::ChannelPosts(Arc::clone(outbox));
    posts.queue_waiting(&post, deadline).await.unwrap();
    post
}

const LATER: i64 = i64::MAX / 2;

/// A waiting post is not settled with its channel's stream: failing the
/// stream settles only the posts queued on it; the waiting ones are listed
/// first-composed first and released once each.
#[tokio::test]
async fn a_waiting_post_is_not_settled_with_its_channels_stream_and_is_released_once() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Arc::new(Outbox::new(
        host(dir.path(), Arc::new(Senders::default())).await,
    ));
    let posts = super::ChannelPosts(Arc::clone(&outbox));
    let written = queued_post(&outbox, "m1").await;
    posts.queue(&written).await.unwrap();
    let first = waiting_post(&outbox, "c1", "m2", LATER).await;
    let second = waiting_post(&outbox, "c1", "m3", LATER).await;
    outbox.pump().await.unwrap();
    assert_eq!(
        (posts.fail_channel("c1", "ch1", "removed").await).unwrap(),
        [written]
    );
    assert_eq!(
        post_state(&outbox, "m2").await,
        Some(SendState::AwaitingKey)
    );
    assert_eq!(
        posts.waiting(Some("c1")).await.unwrap(),
        [first, second.clone()]
    );
    assert!(posts.waiting(Some("c2")).await.unwrap().is_empty());
    assert!(posts.release("m2").await.unwrap());
    assert!(!posts.release("m2").await.unwrap(), "released once");
    assert_eq!(post_state(&outbox, "m2").await, Some(SendState::Queued));
    assert_eq!(posts.waiting(None).await.unwrap(), [second]);
}

/// Past its deadline a waiting post fails with the reason the user reads.
#[tokio::test]
async fn a_post_waiting_past_its_deadline_fails_because_the_key_did_not_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Arc::new(Outbox::new(
        host(dir.path(), Arc::new(Senders::default())).await,
    ));
    let mut changes = outbox.deps().subscribe();
    waiting_post(&outbox, "c1", "m1", 1).await;
    outbox.pump().await.unwrap();
    let change = changes.recv().await.unwrap();
    assert_eq!(change.state, SendState::Failed);
    assert_eq!(
        change.wedge_reason.as_deref(),
        Some(rekindle_outbox::KEY_DID_NOT_ARRIVE)
    );
    assert_eq!(post_state(&outbox, "m1").await, Some(SendState::Failed));
    let posts = super::ChannelPosts(Arc::clone(&outbox));
    assert_eq!(
        posts.post("m1").await.unwrap().map(|p| p.community),
        Some("c1".to_owned()),
        "kept for Retry or Delete"
    );
}

/// Our departure from a community discards its waiting posts, and only its.
#[tokio::test]
async fn our_departure_discards_the_communitys_waiting_posts() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Arc::new(Outbox::new(
        host(dir.path(), Arc::new(Senders::default())).await,
    ));
    let posts = super::ChannelPosts(Arc::clone(&outbox));
    let left = waiting_post(&outbox, "c1", "m1", LATER).await;
    let kept = waiting_post(&outbox, "c2", "m2", LATER).await;
    assert_eq!(posts.discard_community("c1").await.unwrap(), [left]);
    assert_eq!(post_state(&outbox, "m1").await, Some(SendState::Failed));
    assert!(!posts.release("m1").await.unwrap(), "never written");
    assert_eq!(posts.waiting(None).await.unwrap(), [kept]);
}

/// A joiner's repeated bootstrap queues its key once (D16): a second
/// commit under the same client id while its row is kept is a no-op,
/// never a UNIQUE error, and the row keeps its first payload.
#[tokio::test]
async fn a_client_id_queued_once_is_not_queued_again() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Outbox::new(host(dir.path(), Arc::new(Senders::default())).await);
    let entry = || Entry {
        kind: StreamKind::MekTransfer,
        scope: "c:joiner".into(),
        client_id: "MekTransfer:c:joiner:1".into(),
        payload: b"sealed".to_vec(),
        deadline: Deadline::Horizon,
    };
    let once = || outbox.deps().insert_once(outbox.new_row(entry()));
    assert!(once().await.unwrap(), "the first commit stores the row");
    assert!(!once().await.unwrap(), "the repeat is a no-op");
    let row = outbox.deps().row("MekTransfer:c:joiner:1").await.unwrap();
    assert_eq!(row.map(|r| r.payload), Some(b"sealed".to_vec()));
}

/// A media key queued again to the same recipient (a repeated join
/// handshake seals the same key, live finding L-ACK1) is a no-op, never a
/// UNIQUE error, and the row keeps its first payload.
#[tokio::test]
async fn a_media_key_queued_again_is_not_queued_twice() {
    let dir = tempfile::tempdir().unwrap();
    let outbox = Arc::new(Outbox::new(
        host(dir.path(), Arc::new(Senders::default())).await,
    ));
    let keys = super::MediaKeyStreams(Arc::clone(&outbox));
    let client_id = "c:ch/bob/1";
    let first = keys.queue("c:ch", "bob", client_id, b"sealed".to_vec());
    assert!(first.await.unwrap(), "the first queue stores the row");
    let again = keys.queue("c:ch", "bob", client_id, b"resealed".to_vec());
    assert!(!again.await.unwrap(), "the repeat is a no-op");
    let row = outbox.deps().row(client_id).await.unwrap();
    assert_eq!(row.map(|r| r.payload), Some(b"sealed".to_vec()));
}
