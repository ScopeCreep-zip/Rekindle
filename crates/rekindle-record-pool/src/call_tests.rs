use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

fn scope() -> Arc<SessionScope> {
    SessionScope::new("test pool", Arc::new(|_| {}))
}

fn count(seen: Arc<AtomicU32>, value: u32) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        seen.store(value, Ordering::SeqCst);
    })
}

#[tokio::test]
async fn a_claimed_result_reaches_its_caller() {
    let waits = CancellationToken::new();
    let seen = Arc::new(AtomicU32::new(0));
    let got = handover(
        &scope(),
        "call",
        Arc::clone(&seen),
        async { Ok::<u32, ()>(7) },
        Some(count),
        &waits,
        (),
    )
    .await;
    assert_eq!(got, Ok(7));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "a claimed result is not unclaimed"
    );
}

#[tokio::test]
async fn drain_releases_the_wait_and_the_call_runs_on_to_its_handler() {
    let scope = scope();
    let waits = CancellationToken::new();
    let seen = Arc::new(AtomicU32::new(0));
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
    let call = handover(
        &scope,
        "call",
        Arc::clone(&seen),
        async move {
            let _ = finish_rx.await;
            Ok::<u32, ()>(9)
        },
        Some(count),
        &waits,
        (),
    );
    let release = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        waits.cancel();
    };
    let (got, ()) = tokio::join!(call, release);
    assert_eq!(got, Err(()), "the released wait returns at once");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "the call is still running");
    let _ = finish_tx.send(());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        seen.load(Ordering::SeqCst),
        9,
        "the unclaimed result is handled"
    );
}

#[tokio::test]
async fn a_closed_scope_refuses_the_call() {
    let scope = scope();
    scope.shutdown(Duration::from_millis(10)).await.ok();
    let waits = CancellationToken::new();
    let got = handover(
        &scope,
        "call",
        (),
        async { Ok::<u32, ()>(1) },
        None,
        &waits,
        (),
    )
    .await;
    assert_eq!(got, Err(()));
}
