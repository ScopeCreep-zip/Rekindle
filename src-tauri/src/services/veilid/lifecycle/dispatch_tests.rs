use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::work_until_stopped;

/// A worker whose stop sender is gone ends; it never re-polls the
/// closed channel (IC1: two workers spun this way for hours).
#[tokio::test]
async fn a_dropped_stop_sender_ends_the_worker() {
    let (tx, rx) = tokio::sync::watch::channel(false);
    drop(tx);
    let pops = AtomicUsize::new(0);
    let worker = work_until_stopped(
        rx,
        "test",
        || {
            let polled = pops.fetch_add(1, Ordering::SeqCst);
            assert!(polled < 1_000, "the worker spins on a closed stop channel");
            std::future::pending::<()>()
        },
        |()| async {},
    );
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .expect("the worker ends");
    assert_eq!(pops.load(Ordering::SeqCst), 1, "one pass, then the end");
}

/// The flag stops a worker after it handled what it was given.
#[tokio::test]
async fn the_stop_flag_ends_the_worker_after_its_items() {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let (items_tx, items_rx) = tokio::sync::mpsc::unbounded_channel::<u32>();
    let items = tokio::sync::Mutex::new(items_rx);
    let handled = AtomicUsize::new(0);
    items_tx.send(1).expect("send");
    items_tx.send(2).expect("send");
    let (items, counted, flag) = (&items, &handled, &tx);
    let worker = work_until_stopped(
        rx,
        "test",
        move || async move {
            let next = items.lock().await.recv().await;
            if next.is_none() {
                std::future::pending::<()>().await;
            }
            next
        },
        move |_| async move {
            if counted.fetch_add(1, Ordering::SeqCst) == 1 {
                flag.send(true).expect("flag");
            }
        },
    );
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .expect("the worker ends on the flag");
    assert_eq!(handled.load(Ordering::SeqCst), 2);
}
