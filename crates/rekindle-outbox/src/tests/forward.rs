//! Bringing a peer's waiting rows forward (live finding L-FR2).

use super::*;

/// A handshake row backing off after unanswered calls is due at once when
/// the peer's route changes, with its attempts kept; without
/// [`Outbox::bring_forward`] it waits its backoff and the pass skips it.
#[tokio::test]
async fn a_backing_off_handshake_row_is_due_at_once_when_brought_forward() {
    let o = outbox();
    // Each call times out: the row is deferred and backs off.
    script(&o, [AttemptOutcome::Deferred]);
    o.enqueue(entry(StreamKind::FriendHandshake, "bob", "h"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    script(&o, [AttemptOutcome::Deferred]);
    o.enqueue(entry(StreamKind::Dm, "carol", "d"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    let waiting = o.deps().row("h").await.unwrap().unwrap();
    assert!(waiting.next_at > 1_000, "the unanswered row backs off");
    assert_eq!(waiting.attempts, 1);
    // Another peer's stream is not touched.
    let carol = o.deps().row("d").await.unwrap().unwrap();
    assert!(carol.next_at > 1_000);
    assert_eq!(
        o.bring_forward(StreamKind::FriendHandshake, "bob")
            .await
            .unwrap(),
        1
    );
    let due = o.deps().row("h").await.unwrap().unwrap();
    assert_eq!((due.next_at, due.attempts), (1_000, 1));
    assert_eq!(
        o.deps().row("d").await.unwrap().unwrap().next_at,
        carol.next_at
    );
    script(&o, [AttemptOutcome::Receipted]);
    o.pump().await.unwrap();
    let attempted = o.deps().store.lock().attempted.clone();
    assert_eq!(
        attempted,
        ["h", "d", "h"],
        "the brought-forward row was attempted"
    );
}
