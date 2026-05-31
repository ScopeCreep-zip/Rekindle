//! Phase 9 — cold-start event buffer for the subscription dispatch loop.
//!
//! Veilid's `update_callback` fires the moment the API attaches to the
//! network — but the app's downstream consumer (the event pipeline +
//! Tauri emitter) isn't installed until `setup()` finishes. Without a
//! buffer, any events that arrive in that gap are lost.
//!
//! [`ColdStartBuffer`] holds a bounded queue of events while
//! `install_callback` hasn't been called yet. When the consumer is
//! ready, it calls `install_callback` exactly once; the buffer's
//! contents are drained to the consumer in arrival order, and every
//! subsequent event bypasses the buffer entirely.
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 9.

use parking_lot::RwLock;

/// Capacity per the plan — 4096 events bounds memory at startup; if
/// the Veilid attach is so slow that we exceed this, additional events
/// are dropped (with a warn log) rather than ballooning unbounded.
pub const COLD_START_CAPACITY: usize = 4096;

/// Generic over the event type `T` — the plan referenced a
/// `TransportEvent` type that doesn't exist; the actual events flowing
/// through dispatch are typically `veilid_core::VeilidUpdate`. Caller
/// picks T to match the dispatch loop's event shape.
pub struct ColdStartBuffer<T> {
    /// `None` once `install_callback` has been called — any subsequent
    /// `record` returns false immediately. `Some(Vec)` until then.
    buffer: RwLock<Option<Vec<T>>>,
}

impl<T> ColdStartBuffer<T> {
    /// Construct an empty buffer with the plan-specified capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: RwLock::new(Some(Vec::with_capacity(COLD_START_CAPACITY))),
        }
    }

    /// Try to record `ev`. Returns:
    /// - `Ok(())` if the event was buffered (callback not yet installed
    ///   and capacity available).
    /// - `Err(ev)` returning ownership of the event when buffering fails
    ///   (callback already installed OR buffer at capacity). Caller must
    ///   then dispatch the event directly to the live consumer — events
    ///   are NEVER silently dropped at this layer; only the overflow
    ///   case logs a warn for visibility.
    pub fn try_record(&self, ev: T) -> Result<(), T> {
        let mut guard = self.buffer.write();
        if let Some(buf) = guard.as_mut() {
            if buf.len() < COLD_START_CAPACITY {
                buf.push(ev);
                return Ok(());
            }
            tracing::warn!(
                capacity = COLD_START_CAPACITY,
                "cold-start buffer at capacity; event will be dispatched directly without buffering",
            );
        }
        Err(ev)
    }

    /// Install the live consumer callback. Atomically:
    /// - Marks the buffer as closed (subsequent `record` returns false).
    /// - Drains the buffered events to `cb` in arrival order.
    ///
    /// Should be called exactly once per `ColdStartBuffer` instance.
    /// Calling twice no-ops the second time (buffer is already closed).
    pub fn install_callback<F: FnMut(T)>(&self, mut cb: F) {
        let drained = self.buffer.write().take();
        if let Some(drained) = drained {
            let count = drained.len();
            tracing::debug!(count, "cold_start_drain");
            for ev in drained {
                cb(ev);
            }
        }
    }

    /// Diagnostic: number of events currently buffered, or `None` once
    /// the callback has been installed.
    #[must_use]
    pub fn buffered_count(&self) -> Option<usize> {
        self.buffer.read().as_ref().map(Vec::len)
    }

    /// Diagnostic: whether the callback has been installed.
    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.buffer.read().is_none()
    }
}

impl<T> Default for ColdStartBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Events recorded before install are drained in order on install.
    #[test]
    fn record_then_install_drains_in_order() {
        let buf = ColdStartBuffer::<u32>::new();
        assert!(buf.try_record(1).is_ok());
        assert!(buf.try_record(2).is_ok());
        assert!(buf.try_record(3).is_ok());
        assert_eq!(buf.buffered_count(), Some(3));

        let collected = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let c = Arc::clone(&collected);
        buf.install_callback(move |ev| c.lock().push(ev));
        let got = collected.lock().clone();
        assert_eq!(got, vec![1, 2, 3]);
        assert!(buf.is_installed());
    }

    /// After install, `try_record` returns `Err(ev)`; caller recovers
    /// ownership of the event and must dispatch directly.
    #[test]
    fn record_after_install_returns_err_with_event() {
        let buf = ColdStartBuffer::<u32>::new();
        assert!(buf.try_record(1).is_ok());
        buf.install_callback(|_| {});
        assert_eq!(
            buf.try_record(2),
            Err(2),
            "try_record after install must return Err with the original event",
        );
        assert!(buf.is_installed());
    }

    /// Exceeding capacity drops further events (returns false), but the
    /// already-buffered ones survive.
    #[test]
    fn capacity_overflow_drops_excess() {
        let buf = ColdStartBuffer::<u32>::new();
        for i in 0..u32::try_from(COLD_START_CAPACITY).expect("fits in u32") {
            assert!(buf.try_record(i).is_ok(), "should accept event #{i}");
        }
        // Next record exceeds capacity — caller recovers the event.
        assert_eq!(
            buf.try_record(99_999_999),
            Err(99_999_999),
            "overflow event must be returned to caller",
        );
        assert_eq!(buf.buffered_count(), Some(COLD_START_CAPACITY));
    }

    /// Empty buffer drains to zero invocations.
    #[test]
    fn empty_drain_invokes_callback_zero_times() {
        let buf = ColdStartBuffer::<u32>::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        buf.install_callback(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(buf.is_installed());
    }

    /// Calling install_callback twice is idempotent — the second call
    /// is a no-op (buffer already taken).
    #[test]
    fn install_callback_twice_is_noop_second_time() {
        let buf = ColdStartBuffer::<u32>::new();
        assert!(buf.try_record(1).is_ok());
        let count1 = Arc::new(AtomicUsize::new(0));
        let c1 = Arc::clone(&count1);
        buf.install_callback(move |_| {
            c1.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(count1.load(Ordering::Relaxed), 1);

        let count2 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&count2);
        buf.install_callback(move |_| {
            c2.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(count2.load(Ordering::Relaxed), 0, "second install is no-op");
    }

    /// Drain count matches what was recorded (the plan's trace log
    /// invariant: `cold_start_drain count=N`).
    #[test]
    fn drain_count_matches_recorded() {
        let buf = ColdStartBuffer::<&'static str>::new();
        for ev in &["a", "b", "c", "d", "e"] {
            assert!(buf.try_record(*ev).is_ok());
        }
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        buf.install_callback(move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(count.load(Ordering::Relaxed), 5);
    }
}
