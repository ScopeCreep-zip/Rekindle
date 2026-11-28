//! Bounded FIFO between the inbound dispatch loop and the control worker.
//! Work is handled in arrival order. At capacity, the oldest queued
//! notification is dropped and the loss is counted.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use tokio::sync::Notify;

/// At most this many control notifications can wait for the worker.
pub const CONTROL_INGRESS_QUEUE_MAX: usize = 512;

/// A bounded queue for a single control worker.
pub struct ControlIngressQueue<T> {
    inner: Mutex<VecDeque<T>>,
    notify: Notify,
    /// Oldest notifications dropped at capacity.
    pub drops: AtomicU64,
}

impl<T> Default for ControlIngressQueue<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            drops: AtomicU64::new(0),
        }
    }
}

impl<T> ControlIngressQueue<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue the update, evicting the oldest at capacity.
    pub fn push(&self, update: T) {
        {
            let mut q = self.inner.lock();
            if q.len() >= CONTROL_INGRESS_QUEUE_MAX {
                q.pop_front();
                let n = self.drops.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 || n.is_multiple_of(50) {
                    tracing::warn!(
                        dropped_total = n,
                        "control ingress queue full — dropping oldest control event"
                    );
                }
            }
            q.push_back(update);
        }
        self.notify.notify_one();
    }

    /// Await the next update (FIFO). Cancel-safe: a `Notify` permit is
    /// only consumed when the queue really was empty.
    pub async fn pop(&self) -> T {
        loop {
            if let Some(update) = self.inner.lock().pop_front() {
                return update;
            }
            self.notify.notified().await;
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.inner.lock().len()
    }
}

#[cfg(test)]
#[path = "control_ingress_tests.rs"]
mod tests;
