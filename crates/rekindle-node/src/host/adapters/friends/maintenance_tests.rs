use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

fn counting() -> (Arc<AtomicUsize>, impl FnMut() -> std::future::Ready<()>) {
    let passes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&passes);
    (passes, move || {
        counter.fetch_add(1, Ordering::SeqCst);
        std::future::ready(())
    })
}

async fn settle(by: Duration) {
    tokio::time::advance(by).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// Audit D-7: no pass runs before the own-records stage has filled our
/// record, however long it takes, and the first one is a whole tick
/// after it says so.
#[tokio::test(start_paused = true)]
async fn the_maintenance_cannot_mint_before_the_stage_finishes() {
    let (passes, pass) = counting();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let stop = CancellationToken::new();
    let task = tokio::spawn(run_prekeys(wait, stop.clone(), pass));
    settle(PREKEY_TICK * 5).await;
    assert_eq!(passes.load(Ordering::SeqCst), 0, "ran before the stage");
    ready.send(()).unwrap();
    settle(Duration::ZERO).await;
    settle(PREKEY_TICK.checked_sub(Duration::from_secs(1)).unwrap()).await;
    assert_eq!(passes.load(Ordering::SeqCst), 0, "ran at once");
    settle(Duration::from_secs(1)).await;
    assert_eq!(passes.load(Ordering::SeqCst), 1);
    settle(PREKEY_TICK).await;
    assert_eq!(passes.load(Ordering::SeqCst), 2);
    stop.cancel();
    task.await.unwrap();
}

/// A stage that did not hold our record drops the signal: the task ends
/// without a pass.
#[tokio::test(start_paused = true)]
async fn no_record_held_means_no_maintenance() {
    let (passes, pass) = counting();
    let (ready, wait) = tokio::sync::oneshot::channel::<()>();
    drop(ready);
    run_prekeys(wait, CancellationToken::new(), pass).await;
    assert_eq!(passes.load(Ordering::SeqCst), 0);
}
