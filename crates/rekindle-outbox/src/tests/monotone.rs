//! An own row's state only moves forward (D22: `queued < sent < delivered <
//! read`): an attempt settled from the row as it was when the attempt began
//! never publishes, or stores, a state below the one the peer's ack raised
//! it to meanwhile (Linux real-app round 27: `delivered`, then `sent`). The
//! one move down is the user's Retry (`failed → queued`).

use super::*;

/// The states published for `id`, in order.
fn shown(o: &Outbox<Fake>, id: &str) -> Vec<SendState> {
    let s = o.deps().store.lock();
    (s.published.iter())
        .filter(|(c, _)| c == id)
        .map(|(_, st)| *st)
        .collect()
}

fn settled_at(o: &Outbox<Fake>, id: &str) -> Option<SendState> {
    let s = o.deps().store.lock();
    (s.settled.iter().rev())
        .find(|(_, _, c, _)| c == id)
        .map(|(_, _, _, st)| *st)
}

/// Enqueue `id` on `kind`'s stream to bob, and the row as an attempt
/// begun now reads it.
async fn begun(o: &Outbox<Fake>, kind: StreamKind, id: &str) -> OutboxRow {
    o.enqueue(entry(kind, "bob", id), ()).await.unwrap();
    o.deps().row(id).await.unwrap().unwrap()
}

async fn ack(o: &Outbox<Fake>, row: &OutboxRow) {
    let acked = o
        .acknowledge(&row.stream, row.stream_seq, PeerAck::Delivered)
        .await
        .unwrap();
    assert!(acked, "the row waits for its ack");
}

#[tokio::test]
async fn dm_ack_during_an_unanswered_attempt_is_never_followed_by_sent() {
    let o = outbox();
    let row = begun(&o, StreamKind::Dm, "m1").await;
    ack(&o, &row).await;
    let now = o.deps().clock().now_ms_i64();
    o.settle_attempt(&row, AttemptOutcome::Unanswered, now)
        .await
        .unwrap();
    assert_eq!(shown(&o, "m1"), [SendState::Queued, SendState::Delivered]);
    assert_eq!(state_of(&o, "m1"), None, "the row settled");
    assert_eq!(settled_at(&o, "m1"), Some(SendState::Delivered));
}

#[tokio::test]
async fn dm_ack_during_a_reask_of_a_sent_row_is_never_followed_by_sent() {
    let o = outbox();
    let queued = begun(&o, StreamKind::Dm, "m1").await;
    o.mark_landed(&queued, 1, o.deps().clock().now_ms_i64())
        .await
        .unwrap();
    let row = o.deps().row("m1").await.unwrap().unwrap();
    assert_eq!(row.state, SendState::Sent);
    ack(&o, &row).await;
    let now = o.deps().clock().now_ms_i64();
    o.settle_attempt(&row, AttemptOutcome::Unanswered, now)
        .await
        .unwrap();
    assert_eq!(
        shown(&o, "m1"),
        [SendState::Queued, SendState::Sent, SendState::Delivered]
    );
    assert_eq!(settled_at(&o, "m1"), Some(SendState::Delivered));
}

#[tokio::test]
async fn group_ack_during_an_attempt_is_never_followed_by_sent() {
    let o = outbox();
    let row = begun(&o, StreamKind::GroupDm, "g1").await;
    ack(&o, &row).await;
    let now = o.deps().clock().now_ms_i64();
    o.settle_attempt(&row, AttemptOutcome::Sent, now)
        .await
        .unwrap();
    assert_eq!(shown(&o, "g1"), [SendState::Queued, SendState::Delivered]);
    assert_eq!(state_of(&o, "g1"), None);
    assert_eq!(settled_at(&o, "g1"), Some(SendState::Delivered));
}

#[tokio::test]
async fn handshake_ack_during_an_unanswered_attempt_is_never_followed_by_sent() {
    let o = outbox();
    let row = begun(&o, StreamKind::FriendHandshake, "FriendRequest:bob").await;
    o.acknowledge_client("FriendRequest:bob", PeerAck::Delivered)
        .await
        .unwrap();
    let now = o.deps().clock().now_ms_i64();
    o.settle_attempt(&row, AttemptOutcome::Unanswered, now)
        .await
        .unwrap();
    assert_eq!(
        shown(&o, "FriendRequest:bob"),
        [SendState::Queued, SendState::Delivered]
    );
    assert_eq!(state_of(&o, "FriendRequest:bob"), None);
}

#[tokio::test]
async fn an_ack_before_the_landing_settles_delivered_and_the_landing_publishes_nothing() {
    let o = outbox();
    let row = begun(&o, StreamKind::Dm, "m1").await;
    ack(&o, &row).await;
    o.mark_landed(&row, 1, o.deps().clock().now_ms_i64())
        .await
        .unwrap();
    assert_eq!(shown(&o, "m1"), [SendState::Queued, SendState::Delivered]);
    assert_eq!(settled_at(&o, "m1"), Some(SendState::Delivered));
}

#[tokio::test]
async fn a_lower_ack_after_read_publishes_nothing() {
    let o = outbox();
    let row = begun(&o, StreamKind::Dm, "m1").await;
    o.acknowledge(&row.stream, row.stream_seq, PeerAck::Read)
        .await
        .unwrap();
    ack(&o, &row).await;
    assert_eq!(shown(&o, "m1"), [SendState::Queued, SendState::Read]);
}

#[tokio::test]
async fn a_retry_still_moves_failed_to_queued() {
    let o = outbox();
    let row = begun(&o, StreamKind::Dm, "m1").await;
    o.fail(&row, "gone".into()).await.unwrap();
    assert_eq!(state_of(&o, "m1"), Some(SendState::Failed));
    assert!(o.retry("m1").await.unwrap());
    assert_eq!(state_of(&o, "m1"), Some(SendState::Queued));
    assert_eq!(
        shown(&o, "m1"),
        [SendState::Queued, SendState::Failed, SendState::Queued]
    );
}
