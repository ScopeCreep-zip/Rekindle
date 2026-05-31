//! `InboxScanCoordinator` — fans three triggers into one stream of
//! debounced inbox scans.
//!
//! The coordinator is `veilid-core`-free; the concrete [`InboxScanner`]
//! impl lives in `src-tauri` glue so that the friendship crate stays
//! pure logic + tokio. Tests in this module use a deterministic
//! `MockScanner` (see `#[cfg(test)]` block at the bottom).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::interval;

/// 30-second poll backstop. Tuned by the plan; not configurable
/// per-instance (one cadence for the whole app). If a peer's `watch`
/// silently dies, this is the worst-case latency before a friend
/// request becomes visible.
pub const POLL_PERIOD: Duration = Duration::from_secs(30);

/// Coalesce window. Two triggers within this interval collapse to one
/// scan. 500 ms is the plan's value — short enough that a user-clicked
/// "scan now" feels instant, long enough that watch + poll + direct
/// firing back-to-back doesn't triple-scan.
pub const COALESCE: Duration = Duration::from_millis(500);

/// Abstract inbox scanner. Implementations:
/// - Production: src-tauri's `VeilidInboxScanner` (touches `veilid-core`).
/// - Tests: in-process mock with deterministic counter (see tests below).
#[async_trait]
pub trait InboxScanner: Send + Sync + 'static {
    /// Perform one scan of the inbox. Returns the number of entries
    /// processed (for diagnostic tracing). Implementations should
    /// short-circuit if the inbox is empty/unreachable.
    async fn scan(&self) -> Result<u32, ScanError>;
}

/// Three-tier coordinator. Constructed once per logged-in identity;
/// dropped on logout (the `oneshot::Sender<()>` shutdown is the only
/// way to stop the spawned task).
pub struct InboxScanCoordinator<S: InboxScanner> {
    scanner: Arc<S>,
    direct_rx: mpsc::Receiver<()>,
    watch_rx: watch::Receiver<u64>,
}

impl<S: InboxScanner> InboxScanCoordinator<S> {
    /// Build a coordinator. The caller retains the corresponding
    /// `mpsc::Sender<()>` (for direct triggers) and
    /// `watch::Sender<u64>` (for watch-tier wakeups).
    #[must_use]
    pub fn new(
        scanner: Arc<S>,
        direct_rx: mpsc::Receiver<()>,
        watch_rx: watch::Receiver<u64>,
    ) -> Self {
        Self {
            scanner,
            direct_rx,
            watch_rx,
        }
    }

    /// Drive the select-loop until `shutdown` fires. Should be spawned
    /// onto a dedicated tokio task in production. Returns when shutdown
    /// fires OR when all senders for both triggers drop (which would be
    /// a logic error — the caller should hold them until shutdown).
    pub async fn run(mut self, mut shutdown: oneshot::Receiver<()>) {
        let mut poll = interval(POLL_PERIOD);
        // Skip the immediate tick that `interval` produces — we don't
        // want to scan on coordinator startup; the 30-second backstop
        // is exactly that, a *backstop*.
        poll.tick().await;
        // Use a sentinel value past now so the first scan is allowed
        // (otherwise `last.elapsed() < COALESCE` would coalesce-drop
        // the first trigger).
        let mut last_run = Instant::now()
            .checked_sub(COALESCE.saturating_mul(2))
            .unwrap_or_else(Instant::now);
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    tracing::debug!("inbox scan coordinator shutting down");
                    return;
                }
                _ = poll.tick() => {
                    self.maybe_scan(&mut last_run, "poll-30s").await;
                }
                // Detect watch closure explicitly: changed() returns Err
                // when the sender drops. Without this branch the select
                // arm with `Ok(()) = ...` would silently no-match-and-busy-spin.
                changed = self.watch_rx.changed() => {
                    if changed.is_err() {
                        tracing::warn!(
                            "inbox scan coordinator: watch sender dropped without shutdown; exiting"
                        );
                        return;
                    }
                    self.maybe_scan(&mut last_run, "watch-instant").await;
                }
                // Same pattern for the direct mpsc — None means sender dropped.
                recv = self.direct_rx.recv() => {
                    if recv.is_none() {
                        tracing::warn!(
                            "inbox scan coordinator: direct sender dropped without shutdown; exiting"
                        );
                        return;
                    }
                    self.maybe_scan(&mut last_run, "direct").await;
                }
            }
        }
    }

    /// Run a scan if at least `COALESCE` has elapsed since the last
    /// one; otherwise log and skip. `trigger` is a static label for
    /// diagnostics ("poll-30s" / "watch-instant" / "direct").
    async fn maybe_scan(&self, last: &mut Instant, trigger: &'static str) {
        if last.elapsed() < COALESCE {
            tracing::trace!(trigger, "inbox scan coalesced");
            return;
        }
        *last = Instant::now();
        match self.scanner.scan().await {
            Ok(n) => tracing::trace!(trigger, processed = n, "inbox scan complete"),
            Err(e) => tracing::warn!(trigger, error = %e, "inbox scan failed"),
        }
    }
}

/// Errors raised by the [`InboxScanner::scan`] implementation.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// Inbox is not currently reachable (DHT detached, watch not
    /// installed, etc.). The coordinator logs and continues.
    #[error("inbox unavailable: {0}")]
    InboxUnavailable(String),
    /// Any other failure surfaced from the concrete scanner impl.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// In-process scanner that counts invocations and optionally
    /// delays each scan by `delay_ms` to simulate a slow DHT.
    struct MockScanner {
        scans: AtomicUsize,
        delay: Duration,
    }

    impl MockScanner {
        fn new(delay_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                scans: AtomicUsize::new(0),
                delay: Duration::from_millis(delay_ms),
            })
        }
        fn scan_count(&self) -> usize {
            self.scans.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl InboxScanner for MockScanner {
        async fn scan(&self) -> Result<u32, ScanError> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.scans.fetch_add(1, Ordering::Relaxed);
            Ok(0)
        }
    }

    /// Bundle of "things the test wants to drive": senders + shutdown.
    struct Harness {
        direct_tx: mpsc::Sender<()>,
        watch_tx: watch::Sender<u64>,
        shutdown_tx: Option<oneshot::Sender<()>>,
        // Kept-alive to keep the scanner Arc count steady; tests read
        // counts via the outer-scope `scanner` reference, not this field.
        _scanner: Arc<MockScanner>,
        join: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn start(scanner: Arc<MockScanner>) -> Self {
            let (direct_tx, direct_rx) = mpsc::channel(4);
            let (watch_tx, watch_rx) = watch::channel(0u64);
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let coord = InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
            let join = tokio::spawn(coord.run(shutdown_rx));
            Self {
                direct_tx,
                watch_tx,
                shutdown_tx: Some(shutdown_tx),
                _scanner: scanner,
                join,
            }
        }

        async fn shutdown(mut self) {
            let _ = self.shutdown_tx.take().unwrap().send(());
            // run() returns immediately when shutdown fires; wait for
            // the join handle so the test observes a clean exit.
            self.join.await.expect("coordinator task panicked");
        }
    }

    /// Helper: wait up to `total_wait` for `cond` to become true,
    /// polling every 10 ms. Returns the final value of `cond()`.
    /// Used because real-time sleeps under `flavor = "multi_thread"`
    /// are the only reliable way to test this coordinator — the
    /// coalesce check uses `std::time::Instant` which doesn't respond
    /// to `tokio::time::pause`.
    async fn wait_until<F: Fn() -> bool>(cond: F, total_wait: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < total_wait {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }

    /// Direct trigger fires exactly one scan after coordinator startup.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_trigger_fires_one_scan() {
        let scanner = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&scanner));
        h.direct_tx.send(()).await.unwrap();
        let scanner_for_wait = Arc::clone(&scanner);
        assert!(
            wait_until(|| scanner_for_wait.scan_count() == 1, Duration::from_secs(2)).await,
            "direct trigger must invoke scan within 2s; got {}",
            scanner.scan_count(),
        );
        h.shutdown().await;
    }

    /// Watch trigger fires exactly one scan when the sender pushes a
    /// new value.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watch_trigger_fires_one_scan() {
        let scanner = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&scanner));
        h.watch_tx.send(1).unwrap();
        let scanner_for_wait = Arc::clone(&scanner);
        assert!(
            wait_until(|| scanner_for_wait.scan_count() == 1, Duration::from_secs(2)).await,
            "watch trigger must invoke scan",
        );
        h.shutdown().await;
    }

    /// Three triggers within the 500 ms coalesce window must collapse
    /// to ONE scan — the plan's coalesce invariant.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn three_triggers_within_coalesce_collapse_to_one_scan() {
        let scanner = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&scanner));
        // Let the coordinator install its select arms.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Fire all three triggers back-to-back within COALESCE.
        h.direct_tx.send(()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.watch_tx.send(1).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.watch_tx.send(2).unwrap();
        // Give the coordinator time to process all three (well under
        // 500 ms total, so coalesce should drop two).
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            scanner.scan_count(),
            1,
            "three triggers within 500ms must coalesce to one scan",
        );
        h.shutdown().await;
    }

    /// After the coalesce window expires, the next trigger fires a
    /// fresh scan.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trigger_after_coalesce_window_fires_again() {
        let scanner = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&scanner));
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.direct_tx.send(()).await.unwrap();
        // Wait past the coalesce window (real time, since the
        // coordinator's elapsed check uses std::time::Instant).
        tokio::time::sleep(COALESCE + Duration::from_millis(200)).await;
        h.direct_tx.send(()).await.unwrap();
        let scanner_for_wait = Arc::clone(&scanner);
        assert!(
            wait_until(|| scanner_for_wait.scan_count() == 2, Duration::from_secs(2)).await,
            "second trigger after coalesce window must fire fresh scan; count={}",
            scanner.scan_count(),
        );
        h.shutdown().await;
    }

    /// `biased; shutdown` is the highest-priority select arm — even
    /// when a direct trigger is ready alongside shutdown, the
    /// coordinator exits without scanning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_priority_beats_other_triggers() {
        let scanner = MockScanner::new(0);
        let (direct_tx, direct_rx) = mpsc::channel(4);
        let (_watch_tx, watch_rx) = watch::channel(0u64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord = InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
        // Pre-arm both: shutdown signaled first, direct queued.
        let _ = shutdown_tx.send(());
        let _ = direct_tx.send(()).await;
        let join = tokio::spawn(coord.run(shutdown_rx));
        // Coordinator must exit promptly.
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("coordinator must exit on shutdown")
            .expect("task panicked");
        assert_eq!(
            scanner.scan_count(),
            0,
            "shutdown must beat direct trigger when both pending",
        );
    }

    /// Dropping the watch sender without shutdown is detected and the
    /// coordinator exits cleanly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn coordinator_exits_when_watch_sender_drops() {
        let scanner = MockScanner::new(0);
        let (_direct_tx, direct_rx) = mpsc::channel(4);
        let (watch_tx, watch_rx) = watch::channel(0u64);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord = InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
        let join = tokio::spawn(coord.run(shutdown_rx));
        // Sleep so the coordinator enters its select loop.
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(watch_tx);
        // Coordinator's watch arm returns Err → exits cleanly.
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("coordinator must exit when watch sender drops")
            .expect("task panicked");
    }

    /// Dropping the direct sender without shutdown is detected and the
    /// coordinator exits cleanly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn coordinator_exits_when_direct_sender_drops() {
        let scanner = MockScanner::new(0);
        let (direct_tx, direct_rx) = mpsc::channel(4);
        let (_watch_tx, watch_rx) = watch::channel(0u64);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord = InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
        let join = tokio::spawn(coord.run(shutdown_rx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(direct_tx);
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("coordinator must exit when direct sender drops")
            .expect("task panicked");
    }

    /// Scanner errors are logged but don't crash the coordinator;
    /// subsequent triggers still fire.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scanner_error_does_not_stop_coordinator() {
        struct FailScanner {
            count: AtomicUsize,
        }
        #[async_trait]
        impl InboxScanner for FailScanner {
            async fn scan(&self) -> Result<u32, ScanError> {
                self.count.fetch_add(1, Ordering::Relaxed);
                Err(ScanError::InboxUnavailable("test".into()))
            }
        }
        let scanner = Arc::new(FailScanner {
            count: AtomicUsize::new(0),
        });
        let (direct_tx, direct_rx) = mpsc::channel(4);
        let (_watch_tx, watch_rx) = watch::channel(0u64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord = InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
        let join = tokio::spawn(coord.run(shutdown_rx));
        direct_tx.send(()).await.unwrap();
        let s1 = Arc::clone(&scanner);
        wait_until(|| s1.count.load(Ordering::Relaxed) == 1, Duration::from_secs(2)).await;
        tokio::time::sleep(COALESCE + Duration::from_millis(200)).await;
        direct_tx.send(()).await.unwrap();
        let s2 = Arc::clone(&scanner);
        assert!(
            wait_until(|| s2.count.load(Ordering::Relaxed) == 2, Duration::from_secs(2)).await,
            "coordinator must survive scanner error and continue",
        );
        let _ = shutdown_tx.send(());
        join.await.unwrap();
    }
}
