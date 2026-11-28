//! The call watch spans each call with exactly one begin and one end (plan
//! E2.B S8, F11). A separate test binary holding this one test: the watch is
//! process-wide, so in the unit-test binary every concurrently running test's
//! calls were counted too, and the counters could not be read exactly
//! (finding AQ2).

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use rekindle_asql::{CallToken, CallWatch, Connection};

static BEGUN: AtomicUsize = AtomicUsize::new(0);
static ENDED: AtomicUsize = AtomicUsize::new(0);
static QUEUED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The end signal of the call this thread just began, taken by `queued`.
    static PENDING: RefCell<Option<mpsc::Receiver<()>>> = const { RefCell::new(None) };
}

/// A call's token: dropping it ends the call and signals the queuing thread.
struct Ends(mpsc::Sender<()>);

impl Drop for Ends {
    fn drop(&mut self) {
        ENDED.fetch_add(1, Ordering::SeqCst);
        let _ = self.0.send(());
    }
}

/// Blocks the queuing thread until the call's token drops, the way the
/// network simulator's watch does, so the caller resumes with the call ended
/// (the `CallWatch::queued` contract) and the counters are exact.
struct Blocking;

impl CallWatch for Blocking {
    fn begin(&self) -> CallToken {
        BEGUN.fetch_add(1, Ordering::SeqCst);
        let (ended, pending) = mpsc::channel();
        PENDING.with(|slot| *slot.borrow_mut() = Some(pending));
        Some(Box::new(Ends(ended)))
    }

    fn queued(&self) {
        QUEUED.fetch_add(1, Ordering::SeqCst);
        let pending = PENDING
            .with(|slot| slot.borrow_mut().take())
            .expect("queued follows begin on the same thread");
        pending
            .recv_timeout(Duration::from_secs(5))
            .expect("the call's token is dropped once it has run");
    }
}

fn counts() -> (usize, usize, usize) {
    (
        BEGUN.load(Ordering::SeqCst),
        ENDED.load(Ordering::SeqCst),
        QUEUED.load(Ordering::SeqCst),
    )
}

/// The installed watch is told of each call (open, call, detached call,
/// close), and again once it is queued, and its token is dropped only after
/// the call has run: exactly one begin and one end per call.
#[tokio::test]
async fn the_call_watch_spans_each_call() {
    static WATCH: Blocking = Blocking;
    assert!(rekindle_asql::watch_calls(&WATCH));
    assert!(!rekindle_asql::watch_calls(&WATCH), "the first watch stays");

    let conn = Connection::open_in_memory().await.unwrap();
    assert_eq!(counts(), (1, 1, 1), "the open is one call, ended");

    let inside = conn
        .call(|_| {
            Ok::<_, rusqlite::Error>((BEGUN.load(Ordering::SeqCst), ENDED.load(Ordering::SeqCst)))
        })
        .await
        .unwrap();
    assert_eq!(
        inside,
        (2, 1),
        "the call is begun, not ended, while it runs"
    );
    assert_eq!(counts(), (2, 2, 2), "the call ended after it ran");

    conn.call_detached(|_| {}).unwrap();
    assert_eq!(counts(), (3, 3, 3), "the detached call is one call, ended");

    conn.close().await.unwrap();
    assert_eq!(counts(), (4, 4, 4), "the close is one call, ended");
}
