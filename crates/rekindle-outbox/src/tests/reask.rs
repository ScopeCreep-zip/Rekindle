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
    assert!(
        within_first(next_at(&o, "a"), now, first),
        "reset to the first re-ask"
    );
    assert_eq!(
        next_at(&o, "c"),
        10_000_000,
        "another peer's row is untouched"
    );
    // Never later than it was: a sooner re-ask is kept (past the jitter's
    // half step, any new draw is later).
    let reset = next_at(&o, "a");
    advance(&o, first / 2 + 1);
    assert_eq!(o.reask_soon(StreamKind::Dm, "bob").await.unwrap(), 0);
    assert_eq!(next_at(&o, "a"), reset);
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
    assert!(within_first(next_at(&o, "a"), sent_at, first));
}

/// `at` is a first re-ask drawn after `from`: never below the floor
/// `first`, at most half again (`StreamKind::first_reask`).
fn within_first(at: i64, from: i64, first: i64) -> bool {
    (from + first..=from + first * 3 / 2).contains(&at)
}

/// The first re-ask is drawn, not fixed (threat model: no deterministic
/// timing), and never below the floor an ack needs.
#[test]
fn the_first_re_ask_is_jittered_above_its_floor() {
    let o = outbox();
    let first = DM_FIRST_REASK;
    let draws: std::collections::BTreeSet<_> = (0..64)
        .map(|_| StreamKind::Dm.first_reask(o.deps().rng()).unwrap())
        .collect();
    assert!(draws
        .iter()
        .all(|d| (first..=first + first / 2).contains(d)));
    assert!(draws.len() > 1, "drawn, not fixed");
    assert_eq!(StreamKind::MediaKey.first_reask(o.deps().rng()), None);
}

/// Live finding L-DM1: a row whose `app_call` went unanswered (a lost
/// `StreamRecord`) is sent and due again after its backoff without the ack
/// floor, well before the first re-ask, and the wait is drawn.
#[tokio::test]
async fn an_unanswered_call_is_sent_again_soon_not_at_the_first_re_ask() {
    let o = outbox();
    let mut waits = std::collections::BTreeSet::new();
    for id in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        o.enqueue(entry(StreamKind::Dm, id, id), ()).await.unwrap();
        let row = o.deps().row(id).await.unwrap().unwrap();
        let now = o.deps().clock().now_ms_i64();
        o.settle_attempt(&row, AttemptOutcome::Unanswered, now)
            .await
            .unwrap();
        assert_eq!(state_of(&o, id), Some(SendState::Sent), "the frame left");
        let wait = next_at(&o, id) - now;
        assert!(
            (500..=1_000).contains(&wait),
            "first retry is the 1 s step's equal jitter, no floor: {wait}"
        );
        waits.insert(wait);
    }
    assert!(waits.len() > 1, "drawn, not fixed");
    assert_eq!(
        StreamKind::Dm.unanswered_retry(),
        crate::OutboxRetry::Backoff {
            base: std::time::Duration::from_secs(1),
            cap: std::time::Duration::from_secs(300),
            floor: std::time::Duration::ZERO,
        }
    );
}
