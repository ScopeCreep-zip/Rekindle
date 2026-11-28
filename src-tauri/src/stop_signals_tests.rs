use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::{quit_on_stop, StopSource};

/// Signals the test sends, counted as the shell reads them.
struct Fed {
    signals: mpsc::UnboundedReceiver<&'static str>,
    read: Arc<AtomicUsize>,
}

impl StopSource for Fed {
    async fn next(&mut self) -> &'static str {
        match self.signals.recv().await {
            Some(signal) => {
                self.read.fetch_add(1, Ordering::SeqCst);
                signal
            }
            None => std::future::pending().await,
        }
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// A stop signal quits the app; a second one during the shutdown is
/// read and does not quit again.
#[tokio::test]
async fn the_first_stop_signal_quits_once() {
    let (tx, signals) = mpsc::unbounded_channel();
    let read = Arc::new(AtomicUsize::new(0));
    let quits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&quits);
    let task = tokio::spawn(quit_on_stop(
        Fed {
            signals,
            read: Arc::clone(&read),
        },
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
        },
        Duration::from_secs(3600),
        || panic!("no forced end inside the deadline"),
    ));

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(quits.load(Ordering::SeqCst), 0, "no signal, no quit");

    tx.send("SIGTERM").expect("send");
    until("the quit", || quits.load(Ordering::SeqCst) == 1).await;
    tx.send("SIGINT").expect("send");
    tx.send("SIGTERM").expect("send");
    until("the later signals", || read.load(Ordering::SeqCst) == 3).await;
    assert_eq!(
        quits.load(Ordering::SeqCst),
        1,
        "one quit for three signals"
    );
    assert!(!task.is_finished(), "the handler keeps reading");
    task.abort();
}

/// A stop signal ends the process once the deadline passes, even when
/// the graceful shutdown never finishes (IC2); no signal, no end.
#[tokio::test]
async fn a_stop_signal_ends_the_process_after_the_deadline() {
    let (tx, signals) = mpsc::unbounded_channel();
    let forced = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&forced);
    let task = tokio::spawn(quit_on_stop(
        Fed {
            signals,
            read: Arc::new(AtomicUsize::new(0)),
        },
        || {},
        Duration::from_millis(50),
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
        },
    ));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(forced.load(Ordering::SeqCst), 0, "no signal, no forced end");

    tx.send("SIGTERM").expect("send");
    until("the forced end", || forced.load(Ordering::SeqCst) == 1).await;
    task.abort();
}
