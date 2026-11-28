//! Live A/B finding 73 (research `prod_research_batch9.md`): one landing
//! writer, an append never cancelled nor doubled, and the reconcile pass.

use std::time::Duration;

use tokio::sync::Notify;

use super::*;

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap()
}

fn appends(o: &Outbox<Fake>) -> usize {
    o.deps().store.lock().appends
}

fn held(o: &Outbox<Fake>) -> Arc<Notify> {
    let hold = Arc::new(Notify::new());
    o.deps().store.lock().hold = Some(Arc::clone(&hold));
    hold
}

async fn settle_tasks() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// The append blocks past the re-ask: the re-ask resends the hint (a second
/// attempt) and starts no second append; the late completion lands the row.
#[tokio::test]
async fn reask_never_starts_second_append() {
    let o = outbox();
    let hold = held(&o);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    let frame = || AttemptOutcome::Appending(b"f".to_vec());
    script(&o, [frame(), frame(), frame()]);
    let body = async {
        o.pump().await.unwrap();
        settle_tasks().await;
        assert_eq!(appends(&o), 1, "the append is in flight");
        // Within its deadline, nothing is attempted again.
        advance(&o, ms(DM_APPEND_DEADLINE) - 1);
        o.pump().await.unwrap();
        assert_eq!(o.deps().store.lock().attempted.len(), 1);
        advance(&o, ms(DM_FIRST_REASK));
        o.pump().await.unwrap();
        settle_tasks().await;
        assert_eq!(o.deps().store.lock().attempted.len(), 2, "the hint again");
        assert_eq!(appends(&o), 1, "no second append");
        assert_eq!(state_of(&o, "a"), Some(SendState::Queued));
        hold.notify_one();
        settle_tasks().await;
        assert_eq!(state_of(&o, "a"), Some(SendState::Sent), "the late landing");
        // Review LV22 M3: no second job ran after the first, nor waits.
        assert_eq!(appends(&o), 1, "one append in all");
        assert!(!o.stream_busy("dm:bob"), "no job waits");
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}

/// The tick lands the row (its entry on our chain, the ack in) while the
/// append still runs; the append's late completion changes nothing.
#[tokio::test]
async fn late_completion_is_noop() {
    let o = outbox();
    let hold = held(&o);
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    script(&o, [AttemptOutcome::Appending(b"f".to_vec())]);
    let body = async {
        o.pump().await.unwrap();
        settle_tasks().await;
        assert!(o
            .acknowledge(&at.stream, at.stream_seq, PeerAck::Delivered)
            .await
            .unwrap());
        // Within its deadline the tick leaves it to the append.
        o.deps().store.lock().chain.insert(at.stream_seq);
        assert_eq!(o.reconcile().await.unwrap(), 0);
        advance(&o, ms(DM_APPEND_DEADLINE));
        assert_eq!(o.reconcile().await.unwrap(), 1);
        let settled = o.deps().store.lock().settled.clone();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].3, SendState::Delivered);
        let shown = o.deps().store.lock().published.len();
        hold.notify_one();
        settle_tasks().await;
        assert_eq!(o.deps().store.lock().settled.len(), 1, "settled once");
        assert_eq!(
            o.deps().store.lock().published.len(),
            shown,
            "nothing shown"
        );
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}

/// A restart mid-append: the new engine's startup pass lands the row whose
/// entry is on our chain and makes the one whose entry is not due now.
#[tokio::test]
async fn restart_mid_append_reconciles() {
    let o = outbox();
    let a = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    let frame = || AttemptOutcome::Appending(b"f".to_vec());
    script(&o, [frame(), frame()]);
    o.pump().await.unwrap();
    o.deps().store.lock().chain.insert(a.stream_seq);
    let restarted = Outbox::new(o.deps().clone());
    assert_eq!(restarted.reconcile().await.unwrap(), 1);
    assert_eq!(state_of(&restarted, "a"), Some(SendState::Sent));
    let now = restarted.deps().clock.0.load(Ordering::SeqCst);
    let s = restarted.deps().store.lock();
    let b = s.rows.iter().find(|r| r.client_id == "b").unwrap();
    assert_eq!(
        (b.state, b.next_at),
        (SendState::Queued, now),
        "b is due now"
    );
}

/// A row past its deadline, not on our chain, fails at the reconcile and
/// the failure is shown.
#[tokio::test]
async fn lifespan_expiry_visible() {
    let o = outbox();
    let mut e = entry(StreamKind::Dm, "bob", "a");
    e.deadline = Deadline::At(1_000 + 60_000);
    o.enqueue(e, ()).await.unwrap();
    script(&o, [AttemptOutcome::Deferred]);
    o.pump().await.unwrap();
    advance(&o, 60_000);
    assert_eq!(o.reconcile().await.unwrap(), 0);
    let s = o.deps().store.lock();
    let row = s.rows.iter().find(|r| r.client_id == "a").unwrap();
    assert_eq!(row.state, SendState::Failed);
    assert_eq!(row.wedge_reason.as_deref(), Some(DEADLINE_REASON));
    assert_eq!(s.published.last(), Some(&("a".into(), SendState::Failed)));
}

/// The appender is busy while its append runs, though the row left
/// `queued` (discarded mid-append): the friend tick's guard reads this,
/// not the rows (review LV22 M1).
#[tokio::test]
async fn a_stream_stays_busy_while_its_append_runs_without_a_queued_row() {
    let o = outbox();
    let hold = held(&o);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    script(&o, [AttemptOutcome::Appending(b"f".to_vec())]);
    let body = async {
        o.pump().await.unwrap();
        settle_tasks().await;
        o.deps().store.lock().rows.clear();
        assert!(o.stream_busy("dm:bob"), "the append still runs");
        hold.notify_one();
        settle_tasks().await;
        assert!(!o.stream_busy("dm:bob"), "done");
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}

/// E2.5i: a row handed off behind an append in flight is re-hinted at its
/// own `next_at`, and the pass returns that as its next wake, though the
/// store counts the row as blocked behind the in-flight head.
#[tokio::test]
async fn a_rehint_behind_an_append_in_flight_wakes_a_pass() {
    let o = outbox();
    let _hold = held(&o);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    let frame = || AttemptOutcome::Appending(b"f".to_vec());
    script(&o, [frame(), frame(), frame()]);
    let next_at = |id: &str| {
        let s = o.deps().store.lock();
        s.rows.iter().find(|r| r.client_id == id).unwrap().next_at
    };
    let body = async {
        o.pump().await.unwrap();
        settle_tasks().await;
        // b is handed off behind a's append, then a is re-scheduled within
        // its deadline, past b's re-hint.
        advance(
            &o,
            next_at("a") - o.deps().clock.0.load(Ordering::SeqCst) - 1,
        );
        o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
            .await
            .unwrap();
        o.pump().await.unwrap();
        settle_tasks().await;
        advance(&o, 1);
        let wake = o.pump().await.unwrap();
        assert_eq!(appends(&o), 1, "a's append still runs");
        assert!(next_at("b") < next_at("a"), "b's re-hint comes first");
        assert_eq!(wake, Some(next_at("b")), "the pass wakes for b's re-hint");
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}
