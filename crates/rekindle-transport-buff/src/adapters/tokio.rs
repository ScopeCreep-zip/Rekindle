//! [`TokioWake`] — a [`WakeSink`] backed by [`tokio::sync::Notify`].
//!
//! This adapter bridges the runtime-free core with the tokio async runtime.
//! The producer calls `wake()` (non-blocking, any thread) after making
//! progress; the consumer awaits `notified()` in a tokio task.
//!
//! # Cancel safety
//!
//! `Notify::notified()` is cancel-safe. A cancelled `notified()` future
//! does not consume the permit — the next call will see it.
//!
//! # Single-consumer constraint
//!
//! `notify_one()` wakes at most one waiter. If multiple tasks await
//! `notified()` on the same `Notify`, only one is woken per `wake()` call.
//! The spec's `DispatchQueue` and `ReorderRing` are designed for
//! single-consumer drain — this constraint is intentional.

use crate::traits::WakeSink;

/// A [`WakeSink`] backed by [`tokio::sync::Notify`].
///
/// Construct with [`TokioWake::new`] or from an existing `Arc<Notify>`.
///
/// # Example
///
/// ```ignore
/// use rekindle_transport_buff::adapters::tokio::TokioWake;
/// use rekindle_transport_buff::DispatchQueue;
///
/// let wake = TokioWake::new();
/// let queue = DispatchQueue::new(64, wake.clone());
///
/// // Producer (any thread):
/// queue.try_push(0, payload).unwrap();
///
/// // Consumer (tokio task):
/// loop {
///     wake.notified().await;
///     while let Some((seq, item)) = queue.pop() {
///         // process
///     }
/// }
/// ```
#[derive(Clone)]
pub struct TokioWake {
    notify: std::sync::Arc<::tokio::sync::Notify>,
}

impl TokioWake {
    /// Create a new `TokioWake` with a fresh `Notify`.
    pub fn new() -> Self {
        Self {
            notify: std::sync::Arc::new(::tokio::sync::Notify::new()),
        }
    }

    /// Create from an existing `Arc<Notify>` (for sharing with other code).
    pub fn from_notify(notify: std::sync::Arc<::tokio::sync::Notify>) -> Self {
        Self { notify }
    }

    /// Returns a future that completes when `wake()` is called.
    ///
    /// This is the consumer's await point. Call this in a loop:
    /// ```ignore
    /// loop {
    ///     wake.notified().await;
    ///     ring.drain_contiguous(|seq, item| { /* ... */ });
    /// }
    /// ```
    pub fn notified(&self) -> ::tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }

    /// Access the underlying `Notify` for advanced use cases.
    pub fn notify_ref(&self) -> &::tokio::sync::Notify {
        &self.notify
    }
}

impl Default for TokioWake {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeSink for TokioWake {
    fn wake(&self) {
        self.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokio_wake_is_send_sync_clone() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<TokioWake>();
    }

    #[test]
    fn tokio_wake_default_works() {
        let w = TokioWake::default();
        w.wake(); // should not panic
    }
}
