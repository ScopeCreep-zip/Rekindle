//! The `mek_transfer` stream (plan E3.2, D16): a sealed community key is
//! retried until its `app_call` receipt, and a peer's departure (its
//! stream) or ours (every stream of the community) discards its rows.

use super::*;

#[tokio::test]
async fn a_key_transfer_is_retried_until_its_receipt() {
    let o = outbox();
    script(&o, [AttemptOutcome::Deferred, AttemptOutcome::Receipted]);
    o.enqueue(entry(StreamKind::MekTransfer, "c:bob", "t1"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "t1"), Some(SendState::Queued));
    advance(&o, 60_000);
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "t1"), None);
    assert_eq!(
        o.deps().store.lock().published.last(),
        Some(&("t1".into(), SendState::Delivered))
    );
}

#[tokio::test]
async fn a_departure_discards_the_peer_s_rows_and_a_leave_the_community_s() {
    let o = outbox();
    for (scope, id) in [("c:bob", "a"), ("c:carol", "b"), ("d:bob", "c")] {
        o.enqueue(entry(StreamKind::MekTransfer, scope, id), ())
            .await
            .unwrap();
    }
    let kind = StreamKind::MekTransfer;
    assert_eq!(o.discard_streams(kind, "c:bob", "left").await.unwrap(), 1);
    assert_eq!(state_of(&o, "a"), None);
    assert_eq!(state_of(&o, "b"), Some(SendState::Queued));
    assert_eq!(o.discard_streams(kind, "c:", "left").await.unwrap(), 1);
    assert_eq!(state_of(&o, "b"), None);
    assert_eq!(state_of(&o, "c"), Some(SendState::Queued));
}
