//! A dm row's deadline: queued past any number of attempts, failed only at
//! the horizon.

use super::*;

#[tokio::test]
async fn an_offline_peer_keeps_a_dm_queued_past_twenty_attempts_until_the_deadline() {
    let o = outbox();
    script(&o, std::iter::repeat_n(AttemptOutcome::Deferred, 64));
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    // The peer never answers: each due pass defers the row and backs off.
    // No attempt count fails it; it waits at every due time until well
    // past twenty attempts.
    loop {
        o.pump().await.unwrap();
        let row = o.deps().row("a").await.unwrap().unwrap();
        assert_eq!(row.state, SendState::Queued);
        assert!(row.wedge_reason.is_none());
        if row.attempts > 25 {
            break;
        }
        let now = o.deps().clock.0.load(Ordering::SeqCst);
        assert!(row.next_at < row.deadline, "backoff reached the deadline");
        advance(&o, (row.next_at - now).max(1));
    }
    assert!(o.deps().store.lock().attempted.len() > 20);
    // Only the deadline (the delivery horizon) makes it failed.
    let row = o.deps().row("a").await.unwrap().unwrap();
    let now = o.deps().clock.0.load(Ordering::SeqCst);
    assert_eq!(
        row.deadline,
        1_000 + i64::try_from(rekindle_crypto::horizon::DELIVERY_HORIZON.as_millis()).unwrap()
    );
    advance(&o, row.deadline - now);
    o.pump().await.unwrap();
    let row = o.deps().row("a").await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.wedge_reason.as_deref()),
        (SendState::Failed, Some(DEADLINE_REASON))
    );
    // Retry is available and keeps the id.
    assert!(o.retry("a").await.unwrap());
    assert_eq!(state_of(&o, "a"), Some(SendState::Queued));
}
