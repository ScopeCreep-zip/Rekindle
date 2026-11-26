use super::RebuildFlight;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// K triggers while one rebuild runs give that rebuild and one more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn triggers_during_a_rebuild_coalesce_into_one_more() {
    let flight = Arc::new(RebuildFlight::default());
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let runs = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let first = {
        let (flight, lock, runs) = (flight.clone(), lock.clone(), runs.clone());
        tokio::spawn(async move {
            flight
                .run(&lock, || async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).ok();
                    release_rx.await.ok();
                    Some(())
                })
                .await
        })
    };
    started_rx.await.expect("the first rebuild starts");
    let mut waiting = Vec::new();
    for _ in 0..8 {
        let (flight, lock, runs) = (flight.clone(), lock.clone(), runs.clone());
        waiting.push(tokio::spawn(async move {
            flight
                .run(&lock, || async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    Some(())
                })
                .await
        }));
    }
    // Every waiter has taken its ticket before the first finishes.
    while flight.requested.load(Ordering::SeqCst) < 9 {
        tokio::task::yield_now().await;
    }
    release_tx.send(()).expect("the first rebuild waits");
    first.await.expect("first");
    for task in waiting {
        task.await.expect("waiter");
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

/// A failed rebuild covers nothing: the next trigger rebuilds.
#[tokio::test]
async fn a_failed_rebuild_covers_nothing() {
    let flight = RebuildFlight::default();
    let lock = tokio::sync::Mutex::new(());
    assert_eq!(flight.run(&lock, || async { None::<()> }).await, None);
    assert_eq!(flight.run(&lock, || async { Some(1) }).await, Some(1));
}
