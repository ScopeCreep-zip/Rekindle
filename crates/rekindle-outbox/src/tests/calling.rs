//! A `StreamRecord`'s `app_call` is the appender's (its module doc: a pass
//! hands the run's work off and returns without awaiting it): while the
//! call waits for its answer, the rows committed behind it on its stream
//! are attempted, and the call's answer settles its own row only.

use std::time::Duration;

use tokio::sync::Notify;

use super::*;

async fn settle_tasks() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

fn hold(o: &Outbox<Fake>) -> (Arc<Notify>, Arc<Notify>) {
    let (call, append) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let mut s = o.deps().store.lock();
    s.call_hold = Some(Arc::clone(&call));
    s.hold = Some(Arc::clone(&append));
    (call, append)
}

/// A pass, bounded: a pass that awaited the call would never return.
async fn pass(o: &Outbox<Fake>) {
    tokio::time::timeout(Duration::from_secs(2), o.pump())
        .await
        .expect("a pass never awaits a handed-off call")
        .unwrap();
}

#[tokio::test]
async fn rows_behind_an_unanswered_record_call_go_and_its_receipt_settles_it_only() {
    let o = outbox();
    let (call, append) = hold(&o);
    o.enqueue(entry(StreamKind::Dm, "bob", "record"), ())
        .await
        .unwrap();
    let frame = || AttemptOutcome::Appending(b"f".to_vec());
    script(
        &o,
        [AttemptOutcome::Calling(b"r".to_vec()), frame(), frame()],
    );
    let body = async {
        pass(&o).await;
        settle_tasks().await;
        assert_eq!(o.deps().store.lock().calls, 1, "the call is in flight");
        assert_eq!(state_of(&o, "record"), Some(SendState::Queued));
        // Committed while the call is unanswered: the next pass attempts them.
        for id in ["one", "two"] {
            o.enqueue(entry(StreamKind::Dm, "bob", id), ())
                .await
                .unwrap();
        }
        pass(&o).await;
        settle_tasks().await;
        assert_eq!(
            o.deps().store.lock().attempted,
            ["record", "one", "two"],
            "the rows behind the call are attempted, the call's row not again"
        );
        assert_eq!(o.deps().store.lock().calls, 1, "no second call");
        // The call's receipt lands its own row; the appends still run.
        call.notify_one();
        settle_tasks().await;
        assert_eq!(state_of(&o, "record"), Some(SendState::Sent));
        assert_eq!(state_of(&o, "one"), Some(SendState::Queued));
        assert_eq!(state_of(&o, "two"), Some(SendState::Queued));
        append.notify_one();
        settle_tasks().await;
        assert_eq!(state_of(&o, "one"), Some(SendState::Sent));
        assert_eq!(state_of(&o, "two"), Some(SendState::Sent));
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}

/// An unanswered call is the existing short retry: the row counts as sent
/// and is due again after its kind's `unanswered_retry`, never settled.
#[tokio::test]
async fn an_unanswered_record_call_is_retried_never_settled() {
    let o = outbox();
    o.enqueue(entry(StreamKind::Dm, "bob", "record"), ())
        .await
        .unwrap();
    script(&o, [AttemptOutcome::Calling(b"r".to_vec())]);
    o.deps()
        .store
        .lock()
        .call_outcomes
        .push_back(AttemptOutcome::Unanswered);
    let body = async {
        pass(&o).await;
        settle_tasks().await;
        let s = o.deps().store.lock();
        let row = s.rows.iter().find(|r| r.client_id == "record").unwrap();
        assert_eq!(row.state, SendState::Sent);
        assert!(row.next_at > 1_000, "due again after the short retry");
        assert!(s.settled.is_empty(), "an unanswered call settles nothing");
        assert!(s.changes.last().unwrap().retry.is_some(), "shown retrying");
    };
    tokio::select! {
        () = o.append_loop() => unreachable!("the appender runs until its scope ends"),
        () = body => {}
    }
}
