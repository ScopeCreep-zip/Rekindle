//! The single-flight re-resolve (live finding CR2), with a counting stub
//! for the profile read.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use super::{Resolve, ResolveFlights, RESOLVE_BACKOFF};

/// Counts the profile reads; each read waits for `release` (at most a
/// minute of paused time, so a send that should not read never hangs the
/// test), then answers `ok`.
struct ProfileStub {
    reads: AtomicUsize,
    release: Notify,
    ok: bool,
}

impl ProfileStub {
    fn new(ok: bool) -> Arc<Self> {
        Arc::new(Self {
            reads: AtomicUsize::new(0),
            release: Notify::new(),
            ok,
        })
    }

    async fn read_import_send(&self) -> bool {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let _ = tokio::time::timeout(Duration::from_secs(60), self.release.notified()).await;
        self.ok
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[tokio::test(start_paused = true)]
async fn concurrent_video_sends_read_the_profile_once() {
    let flights = Arc::new(ResolveFlights::default());
    let stub = ProfileStub::new(true);
    let leader = {
        let (flights, stub) = (Arc::clone(&flights), Arc::clone(&stub));
        tokio::spawn(async move {
            flights
                .resolve("peer", false, || stub.read_import_send())
                .await
        })
    };
    tokio::task::yield_now().await;
    let mut others = Vec::new();
    for _ in 0..30 {
        others.push(
            flights
                .resolve("peer", false, || stub.read_import_send())
                .await,
        );
    }
    stub.release.notify_one();
    assert_eq!(leader.await.unwrap(), Resolve::Led(true));
    assert!(others.iter().all(|r| *r == Resolve::NotResolved));
    assert_eq!(stub.reads(), 1, "one profile read for 31 sends");
}

#[tokio::test(start_paused = true)]
async fn a_call_signal_sends_once_the_leader_lands() {
    let flights = Arc::new(ResolveFlights::default());
    let stub = ProfileStub::new(true);
    let leader = {
        let (flights, stub) = (Arc::clone(&flights), Arc::clone(&stub));
        tokio::spawn(async move {
            flights
                .resolve("peer", false, || stub.read_import_send())
                .await
        })
    };
    tokio::task::yield_now().await;
    let signal = {
        let (flights, stub) = (Arc::clone(&flights), Arc::clone(&stub));
        tokio::spawn(async move {
            flights
                .resolve("peer", true, || stub.read_import_send())
                .await
        })
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    stub.release.notify_one();
    assert_eq!(leader.await.unwrap(), Resolve::Led(true));
    assert_eq!(signal.await.unwrap(), Resolve::Landed);
    assert_eq!(stub.reads(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_resolve_backs_the_peer_off() {
    let flights = ResolveFlights::default();
    let stub = ProfileStub::new(false);
    stub.release.notify_one();
    let first = flights
        .resolve("peer", true, || stub.read_import_send())
        .await;
    assert_eq!(first, Resolve::Led(false));
    let during = flights
        .resolve("peer", true, || stub.read_import_send())
        .await;
    assert_eq!(during, Resolve::NotResolved);
    assert_eq!(stub.reads(), 1, "no read while backing off");
    tokio::time::sleep(RESOLVE_BACKOFF).await;
    stub.release.notify_one();
    let after = flights
        .resolve("peer", true, || stub.read_import_send())
        .await;
    assert_eq!(after, Resolve::Led(false));
    assert_eq!(stub.reads(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_flight_is_cleared() {
    let flights = Arc::new(ResolveFlights::default());
    let stub = ProfileStub::new(true);
    let leader = {
        let (flights, stub) = (Arc::clone(&flights), Arc::clone(&stub));
        tokio::spawn(async move {
            flights
                .resolve("peer", false, || stub.read_import_send())
                .await
        })
    };
    tokio::task::yield_now().await;
    leader.abort();
    let _ = leader.await;
    stub.release.notify_one();
    let next = flights
        .resolve("peer", false, || stub.read_import_send())
        .await;
    assert_eq!(next, Resolve::Led(true));
    assert_eq!(stub.reads(), 2);
}
