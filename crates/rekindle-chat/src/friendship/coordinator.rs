//! InboxScanCoordinator — fans three triggers into one stream of
//! debounced inbox scans.
//!
//! Triggers:
//! 1. `watch_rx` (`watch::Receiver<u64>`) — DHT ValueChanged on inbox
//! 2. `poll` (30s interval) — backstop for dead watches
//! 3. `direct_rx` (`mpsc::Receiver<()>`) — user-initiated / friend-ack
//!
//! 500ms coalesce window. `biased` select: shutdown > poll > watch > direct.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::interval;

use super::scanner::InboxScanner;

/// 30-second poll backstop.
pub const POLL_PERIOD: Duration = Duration::from_secs(30);

/// Coalesce window. Two triggers within this interval collapse to one scan.
pub const COALESCE: Duration = Duration::from_millis(500);

/// Three-tier coordinator. Constructed once per logged-in identity.
/// Consumed by `run()` — the caller holds the trigger senders and
/// shutdown sender.
pub struct InboxScanCoordinator<S: InboxScanner> {
    scanner: Arc<S>,
    direct_rx: mpsc::Receiver<()>,
    watch_rx: watch::Receiver<u64>,
}

impl<S: InboxScanner> InboxScanCoordinator<S> {
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

    /// Drive the select-loop until shutdown fires or all senders drop.
    pub async fn run(mut self, mut shutdown: oneshot::Receiver<()>) {
        let mut poll = interval(POLL_PERIOD);
        poll.tick().await; // skip immediate tick

        // Sentinel past now so the first trigger is allowed through.
        let mut last_run = Instant::now()
            .checked_sub(COALESCE.saturating_mul(2))
            .unwrap_or_else(Instant::now);

        loop {
            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    tracing::debug!("inbox coordinator: shutdown");
                    return;
                }

                _ = poll.tick() => {
                    self.maybe_scan(&mut last_run, "poll-30s").await;
                }

                changed = self.watch_rx.changed() => {
                    if changed.is_err() {
                        tracing::warn!("inbox coordinator: watch sender dropped");
                        return;
                    }
                    self.maybe_scan(&mut last_run, "watch").await;
                }

                recv = self.direct_rx.recv() => {
                    if recv.is_none() {
                        tracing::warn!("inbox coordinator: direct sender dropped");
                        return;
                    }
                    self.maybe_scan(&mut last_run, "direct").await;
                }
            }
        }
    }

    async fn maybe_scan(&self, last: &mut Instant, trigger: &'static str) {
        if last.elapsed() < COALESCE {
            tracing::trace!(trigger, "inbox scan coalesced");
            return;
        }
        *last = Instant::now();
        match self.scanner.scan().await {
            Ok(n) => tracing::trace!(trigger, processed = n, "inbox scan ok"),
            Err(e) => tracing::warn!(trigger, error = %e, "inbox scan failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::friendship::scanner::ScanError;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        fn count(&self) -> usize {
            self.scans.load(Ordering::Relaxed)
        }
    }

    #[async_trait::async_trait]
    impl InboxScanner for MockScanner {
        async fn scan(&self) -> Result<u32, ScanError> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.scans.fetch_add(1, Ordering::Relaxed);
            Ok(0)
        }
    }

    struct Harness {
        direct_tx: mpsc::Sender<()>,
        watch_tx: watch::Sender<u64>,
        shutdown_tx: Option<oneshot::Sender<()>>,
        join: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        fn start(scanner: Arc<MockScanner>) -> Self {
            let (direct_tx, direct_rx) = mpsc::channel(4);
            let (watch_tx, watch_rx) = watch::channel(0u64);
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let coord =
                InboxScanCoordinator::new(Arc::clone(&scanner), direct_rx, watch_rx);
            let join = tokio::spawn(coord.run(shutdown_rx));
            Self {
                direct_tx,
                watch_tx,
                shutdown_tx: Some(shutdown_tx),
                join,
            }
        }

        async fn shutdown(mut self) {
            let _ = self.shutdown_tx.take().unwrap().send(());
            self.join.await.expect("coordinator panicked");
        }
    }

    async fn wait_until(cond: impl Fn() -> bool, max: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < max {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_trigger_fires_one_scan() {
        let s = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&s));
        h.direct_tx.send(()).await.unwrap();
        assert!(wait_until(|| s.count() == 1, Duration::from_secs(2)).await);
        h.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watch_trigger_fires_one_scan() {
        let s = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&s));
        h.watch_tx.send(1).unwrap();
        assert!(wait_until(|| s.count() == 1, Duration::from_secs(2)).await);
        h.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn three_triggers_coalesce_to_one() {
        let s = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&s));
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.direct_tx.send(()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.watch_tx.send(1).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.watch_tx.send(2).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(s.count(), 1, "three triggers within 500ms must coalesce");
        h.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trigger_after_coalesce_fires_again() {
        let s = MockScanner::new(0);
        let h = Harness::start(Arc::clone(&s));
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.direct_tx.send(()).await.unwrap();
        tokio::time::sleep(COALESCE + Duration::from_millis(200)).await;
        h.direct_tx.send(()).await.unwrap();
        assert!(wait_until(|| s.count() == 2, Duration::from_secs(2)).await);
        h.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_beats_pending_trigger() {
        let s = MockScanner::new(0);
        let (direct_tx, direct_rx) = mpsc::channel(4);
        let (_watch_tx, watch_rx) = watch::channel(0u64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord =
            InboxScanCoordinator::new(Arc::clone(&s), direct_rx, watch_rx);
        let _ = shutdown_tx.send(());
        let _ = direct_tx.send(()).await;
        let join = tokio::spawn(coord.run(shutdown_rx));
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("must exit")
            .expect("panicked");
        assert_eq!(s.count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exits_when_watch_sender_drops() {
        let s = MockScanner::new(0);
        let (_direct_tx, direct_rx) = mpsc::channel(4);
        let (watch_tx, watch_rx) = watch::channel(0u64);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord =
            InboxScanCoordinator::new(Arc::clone(&s), direct_rx, watch_rx);
        let join = tokio::spawn(coord.run(shutdown_rx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(watch_tx);
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("must exit")
            .expect("panicked");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scanner_error_does_not_stop_coordinator() {
        struct FailScanner(AtomicUsize);
        #[async_trait::async_trait]
        impl InboxScanner for FailScanner {
            async fn scan(&self) -> Result<u32, ScanError> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(ScanError::InboxUnavailable("test".into()))
            }
        }
        let s = Arc::new(FailScanner(AtomicUsize::new(0)));
        let (direct_tx, direct_rx) = mpsc::channel(4);
        let (_watch_tx, watch_rx) = watch::channel(0u64);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let coord =
            InboxScanCoordinator::new(Arc::clone(&s), direct_rx, watch_rx);
        let join = tokio::spawn(coord.run(shutdown_rx));
        direct_tx.send(()).await.unwrap();
        wait_until(
            || s.0.load(Ordering::Relaxed) == 1,
            Duration::from_secs(2),
        )
        .await;
        tokio::time::sleep(COALESCE + Duration::from_millis(200)).await;
        direct_tx.send(()).await.unwrap();
        assert!(wait_until(
            || s.0.load(Ordering::Relaxed) == 2,
            Duration::from_secs(2),
        )
        .await);
        let _ = shutdown_tx.send(());
        join.await.unwrap();
    }
}
