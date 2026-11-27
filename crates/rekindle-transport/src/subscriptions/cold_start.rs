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
/// through dispatch are the ports' `PortEvent`s. Caller picks T to match
/// the dispatch loop's event shape.
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
#[path = "cold_start_tests.rs"]
mod tests;
