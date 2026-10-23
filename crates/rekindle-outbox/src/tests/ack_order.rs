//! Live A/B finding 42: a row's landing and the peer's ack are two inputs
//! to one row, and it settles `delivered` only when both are in, in
//! either order; a receipt is both at once, and an awaiting row waits for
//! its ack by client id.

use super::*;

fn settled(o: &Outbox<Fake>) -> Vec<(String, SendState)> {
    let s = o.deps().store.lock();
    s.settled
        .iter()
        .map(|(_, _, id, state)| (id.clone(), *state))
        .collect()
}

#[tokio::test]
async fn an_ack_before_the_landing_waits_for_it() {
    let o = outbox();
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    // The peer acks from the hint while the append is still in flight.
    assert!(o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Delivered)
        .await
        .unwrap());
    assert_eq!(state_of(&o, "a"), Some(SendState::Queued), "not landed yet");
    assert!(settled(&o).is_empty());
    // The append did not land this time: the ack stays on the row.
    script(&o, [AttemptOutcome::Deferred, AttemptOutcome::Sent]);
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Queued));
    advance(&o, 600_000);
    o.pump().await.unwrap();
    assert_eq!(settled(&o), [("a".to_owned(), SendState::Delivered)]);
    let published = o.deps().store.lock().published.clone();
    let states: Vec<_> = published.into_iter().map(|(_, s)| s).collect();
    assert_eq!(states, [SendState::Queued, SendState::Delivered]);
}

#[tokio::test]
async fn a_landing_before_the_ack_settles_at_the_ack() {
    let o = outbox();
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent), "landed, waiting");
    assert!(settled(&o).is_empty());
    assert!(o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Delivered)
        .await
        .unwrap());
    assert_eq!(settled(&o), [("a".to_owned(), SendState::Delivered)]);
}

#[tokio::test]
async fn the_higher_early_ack_is_kept() {
    let o = outbox();
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    for ack in [PeerAck::Read, PeerAck::Delivered] {
        o.acknowledge(&at.stream, at.stream_seq, ack).await.unwrap();
    }
    o.pump().await.unwrap();
    assert_eq!(settled(&o), [("a".to_owned(), SendState::Read)]);
}

#[tokio::test]
async fn a_handshake_frame_settles_sent_and_a_receipt_delivered() {
    let o = outbox();
    o.enqueue(entry(StreamKind::FriendHandshake, "bob", "accept"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::FriendHandshake, "bob", "request"), ())
        .await
        .unwrap();
    script(&o, [AttemptOutcome::Sent, AttemptOutcome::Receipted]);
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "accept"), None, "settled and gone");
    assert_eq!(state_of(&o, "request"), None);
    assert_eq!(o.unsettled().await.unwrap(), 0);
}

#[tokio::test]
async fn an_awaiting_row_waits_for_its_ack_by_client_id() {
    let o = outbox();
    o.enqueue(
        entry(StreamKind::FriendHandshake, "bob", "unfriended:bob"),
        (),
    )
    .await
    .unwrap();
    script(&o, [AttemptOutcome::Awaiting]);
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "unfriended:bob"), Some(SendState::Sent));
    assert!(o
        .acknowledge_client("unfriended:bob", PeerAck::Delivered)
        .await
        .unwrap());
    assert_eq!(state_of(&o, "unfriended:bob"), None);
    assert!(!o
        .acknowledge_client("unfriended:bob", PeerAck::Delivered)
        .await
        .unwrap());
}
