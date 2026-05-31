//! Watch-tier trigger primitive.
//!
//! Owns the counter + sender + disable-deadline that previously lived
//! as five separate fields on `AppState`. The Tauri-side adapter
//! installs the watch::Sender after `spawn_coordinator` creates the
//! channel pair; the DHT-watch dispatch path calls `fire()` on each
//! ValueChange.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::watch;

/// Counter + sender + disable-deadline for the watch tier.
///
/// `Default` produces an empty trigger with no sender installed; call
/// `install_sender` when `spawn_coordinator` creates the channel.
/// `clear` drops the sender (logout). `fire` is called by the Veilid
/// DHT dispatch path on each inbox ValueChange; it no-ops when no
/// coordinator is running or the disable deadline is in the future.
pub struct WatchTrigger {
    counter: AtomicU64,
    tx: Mutex<Option<watch::Sender<u64>>>,
    disabled_until: Mutex<Option<Instant>>,
}

impl WatchTrigger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            tx: Mutex::new(None),
            disabled_until: Mutex::new(None),
        }
    }

    /// Reset the counter (so a re-login starts from 0; otherwise the new
    /// coordinator's `watch_rx` initial value (0) would never differ
    /// from a stale counter, and the first trigger would no-op) and
    /// install the channel's sender.
    pub fn install_sender(&self, tx: watch::Sender<u64>) {
        self.counter.store(0, Ordering::Relaxed);
        *self.tx.lock() = Some(tx);
    }

    /// Drop the sender AND clear the dev-disable deadline — called on
    /// logout. Mirrors the pre-Phase-12 cleanup which explicitly reset
    /// both `friendship_watch_tx` and `friendship_watch_disabled_until`
    /// so a re-login starts from a clean state (no stale deadline that
    /// would suppress watch-tier scans on the new identity).
    pub fn clear(&self) {
        *self.tx.lock() = None;
        *self.disabled_until.lock() = None;
    }

    /// Fire the trigger. Silently drops if:
    /// 1. `disabled_until` is set and the deadline is in the future
    ///    (the `dev_disable_watch` test scenario).
    /// 2. No coordinator is running (e.g. user is logged out — `tx`
    ///    is `None` between logout and the next login).
    pub fn fire(&self) {
        let disabled_until = *self.disabled_until.lock();
        if let Some(deadline) = disabled_until {
            if Instant::now() < deadline {
                tracing::trace!(
                    until = ?deadline,
                    "friendship watch trigger dropped — watch tier dev-disabled",
                );
                return;
            }
        }
        let tx_guard = self.tx.lock();
        let Some(tx) = tx_guard.as_ref() else {
            tracing::trace!("friendship watch trigger dropped — no coordinator running");
            return;
        };
        // Monotonically-increasing counter so each send is a fresh
        // value (`watch::Sender::send` only notifies on value change).
        let n = self
            .counter
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if let Err(e) = tx.send(n) {
            tracing::trace!(error = %e, "friendship watch trigger: all receivers dropped");
        }
    }

    /// Dev-only: disable the watch tier for the given duration. While
    /// the deadline is in the future, `fire` no-ops; only the 30 s poll
    /// backstop + direct triggers deliver scans.
    pub fn disable_for(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        *self.disabled_until.lock() = Some(deadline);
        let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            duration_ms,
            "friendship watch tier disabled until {deadline:?}",
        );
    }
}

impl Default for WatchTrigger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_without_sender_is_noop() {
        let t = WatchTrigger::new();
        t.fire(); // no panic
    }

    #[test]
    fn install_then_fire_increments_counter_and_sends() {
        let t = WatchTrigger::new();
        let (tx, mut rx) = watch::channel(0u64);
        t.install_sender(tx);
        assert_eq!(*rx.borrow(), 0);
        t.fire();
        assert_eq!(*rx.borrow_and_update(), 1);
        t.fire();
        assert_eq!(*rx.borrow_and_update(), 2);
    }

    #[test]
    fn disable_for_blocks_fire_until_deadline() {
        let t = WatchTrigger::new();
        let (tx, rx) = watch::channel(0u64);
        t.install_sender(tx);
        t.disable_for(Duration::from_millis(50));
        t.fire();
        // Counter must not have been observed by the receiver yet
        assert_eq!(*rx.borrow(), 0);
        std::thread::sleep(Duration::from_millis(60));
        t.fire();
        assert_eq!(*rx.borrow(), 1);
    }

    #[test]
    fn clear_drops_sender_and_resets_deadline() {
        let t = WatchTrigger::new();
        let (tx, _rx) = watch::channel(0u64);
        t.install_sender(tx);
        t.disable_for(Duration::from_secs(3600));
        t.clear();
        // Sender dropped: fire() should no-op even though the disable
        // deadline would otherwise have suppressed it.
        t.fire();
        // Disable deadline cleared: installing a fresh sender + firing
        // should send immediately, not wait an hour.
        let (tx2, mut rx2) = watch::channel(0u64);
        t.install_sender(tx2);
        t.fire();
        assert_eq!(*rx2.borrow_and_update(), 1);
    }

    #[test]
    fn install_sender_resets_counter() {
        let t = WatchTrigger::new();
        let (tx1, _rx1) = watch::channel(0u64);
        t.install_sender(tx1);
        t.fire();
        t.fire();
        let (tx2, mut rx2) = watch::channel(0u64);
        t.install_sender(tx2);
        t.fire();
        assert_eq!(*rx2.borrow_and_update(), 1);
    }
}
