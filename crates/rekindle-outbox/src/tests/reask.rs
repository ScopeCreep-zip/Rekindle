//! E2.5g: a landed, unacked row's re-ask backoff resets when its peer is
//! heard from again ([`Outbox::reask_soon`]).

use super::*;

fn next_at(o: &Outbox<Fake>, id: &str) -> i64 {
    let s = o.deps().store.lock();
    s.rows.iter().find(|r| r.client_id == id).unwrap().next_at
}

fn set_next_at(o: &Outbox<Fake>, id: &str, at: i64) {
    let mut s = o.deps().store.lock();
    s.rows
        .iter_mut()
        .find(|r| r.client_id == id)
        .unwrap()
        .next_at = at;
}

#[tokio::test]
async fn a_peer_heard_again_brings_its_landed_rows_re_ask_forward_only() {
    let first = i64::try_from(DM_FIRST_REASK.as_millis()).unwrap();
    let o = outbox();
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "cat", "c"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
    // Its backoff grew far past the first re-ask.
    set_next_at(&o, "a", 10_000_000);
    set_next_at(&o, "c", 10_000_000);
    advance(&o, 5_000);
    let now = 6_000;
    assert_eq!(o.reask_soon(StreamKind::Dm, "bob").await.unwrap(), 1);
    assert_eq!(next_at(&o, "a"), now + first, "reset to the first re-ask");
    assert_eq!(
        next_at(&o, "c"),
        10_000_000,
        "another peer's row is untouched"
    );
    // Never later than it was: a sooner re-ask is kept.
    advance(&o, 1_000);
    assert_eq!(o.reask_soon(StreamKind::Dm, "bob").await.unwrap(), 0);
    assert_eq!(next_at(&o, "a"), now + first);
    // A kind with no re-ask, and a row not yet landed, are untouched.
    assert_eq!(o.reask_soon(StreamKind::DhtWrite, "bob").await.unwrap(), 0);
    o.enqueue(entry(StreamKind::Dm, "bob", "q"), ())
        .await
        .unwrap();
    set_next_at(&o, "q", 10_000_000);
    o.reask_soon(StreamKind::Dm, "bob").await.unwrap();
    assert_eq!(state_of(&o, "q"), Some(SendState::Queued));
    assert_eq!(next_at(&o, "q"), 10_000_000);
}

/// Live A/B finding 73c: a row's first re-ask runs from when its hint left,
/// not from its append's landing, however late that comes.
#[tokio::test]
async fn the_first_re_ask_runs_from_the_hint_not_the_landing() {
    let first = i64::try_from(DM_FIRST_REASK.as_millis()).unwrap();
    let o = outbox();
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    let row = o.deps().row("a").await.unwrap().unwrap();
    // The hint left at 1 000; the append landed 20 s later.
    let sent_at = 1_000;
    advance(&o, 20_000);
    o.settle_attempt(&row, AttemptOutcome::Sent, sent_at)
        .await
        .unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
    assert_eq!(next_at(&o, "a"), sent_at + first);
}
