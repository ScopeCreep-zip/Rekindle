//! Plan §3.0 lane 1: a receipted send that goes unanswered publishes, on
//! every attempt, that it is not delivered yet and still being tried (its
//! attempt count and next attempt); the first answer clears it.

use rekindle_types::message::DeliveryRetry;

use super::*;

fn last_change(o: &Outbox<Fake>, id: &str) -> StateChange {
    let s = o.deps().store.lock();
    s.changes
        .iter()
        .rev()
        .find(|c| c.client_id == id)
        .cloned()
        .unwrap()
}

#[tokio::test]
async fn every_unanswered_attempt_publishes_its_retry_and_the_answer_clears_it() {
    let o = outbox();
    o.enqueue(
        entry(StreamKind::FriendHandshake, "bob", "FriendRequest:bob"),
        (),
    )
    .await
    .unwrap();
    for attempt in 1..=3_u32 {
        let row = o.deps().row("FriendRequest:bob").await.unwrap().unwrap();
        let now = o.deps().clock().now_ms_i64();
        o.settle_attempt(&row, AttemptOutcome::Unanswered, now)
            .await
            .unwrap();
        let stored = o.deps().row("FriendRequest:bob").await.unwrap().unwrap();
        let change = last_change(&o, "FriendRequest:bob");
        assert_eq!(change.state, SendState::Sent, "attempt {attempt}");
        assert_eq!(
            change.retry,
            Some(DeliveryRetry {
                attempts: attempt,
                next_attempt_at: stored.next_at,
            }),
            "attempt {attempt} publishes its count and next attempt"
        );
        advance(&o, 60_000);
    }
    let retries = o
        .deps()
        .store
        .lock()
        .changes
        .iter()
        .filter(|c| c.retry.is_some())
        .count();
    assert_eq!(retries, 3, "one retry change per unanswered attempt");
    // The first answer: delivered, and the retry is gone.
    let row = o.deps().row("FriendRequest:bob").await.unwrap().unwrap();
    let now = o.deps().clock().now_ms_i64();
    o.settle_attempt(&row, AttemptOutcome::Receipted, now)
        .await
        .unwrap();
    let change = last_change(&o, "FriendRequest:bob");
    assert_eq!(change.state, SendState::Delivered);
    assert_eq!(change.retry, None, "the answer clears the retry");
}
