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
