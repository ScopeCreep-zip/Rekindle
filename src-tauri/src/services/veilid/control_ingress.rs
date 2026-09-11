//! Piece 6 — inbound control-event decoupling: the Veilid pipeline's
//! second bodyguard.
//!
//! The Veilid update dispatch loop is SERIAL. Post-cold-start it used
//! to `.await` the SLOW identity-dependent handlers inline —
//! `ValueChange` (DHT read → DB → decrypt) and `AppCall` (network
//! round-trips). While the loop awaited one, incoming media
//! `AppMessage`s piled up in the node's bounded 4096-deep update
//! channel (`crates/rekindle-protocol/src/node.rs`) and overflowed: a
//! live bidirectional call measured 388 media drops + 5 ValueChange
//! drops in ~34 s.
//!
//! This queue is the exact sibling of [`GossipIngressQueue`]: the
//! dispatch loop classifies + pushes and RETURNS; one worker (spawned
//! by `lifecycle::dispatch`) drains in FIFO order via
//! `handle_veilid_update`. `AppMessage` never enters here — its handler
//! is already sync-fast (media `try_send` / gossip-queue push).
//!
//! Ordering safety: media (`AppMessage`, dispatched inline) and control
//! (`ValueChange`/`AppCall`, this queue) are independent inbound streams
//! with no cross-ordering dependency — decoupling them reorders nothing
//! that matters. Among control events the single worker preserves FIFO;
//! governance `ValueChange`s are CRDT-merged (order-independent by
//! design) and presence `ValueChange`s carry `last_heartbeat` and are
//! staleness-checked. That is also why the drop-oldest overflow below is
//! safe: a shed DHT update is re-read on the next watch tick.
//!
//! [`GossipIngressQueue`]: crate::services::veilid::ingress_queue::GossipIngressQueue

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use tokio::sync::Notify;
use veilid_core::VeilidUpdate;

/// Bounded depth. `ValueChange`/`AppCall` are LOW-rate versus media
/// (watch ticks, membership/governance edits, call signaling), so 512
/// is minutes of steady control traffic — deep enough to ride out a
/// burst while one slow handler runs, shallow enough to bound the
/// worst-case processing lag of a stalled worker.
pub const CONTROL_INGRESS_QUEUE_MAX: usize = 512;

/// FIFO queue between the Veilid dispatch loop and the control worker.
///
/// Elements are owned `VeilidUpdate`s (`ValueChange`/`AppCall`) awaiting
/// `handle_veilid_update`. Unlike the gossip queue there is nothing to
/// pre-decode, so the raw update is stored as-is.
#[derive(Default)]
pub struct ControlIngressQueue {
    inner: Mutex<VecDeque<VeilidUpdate>>,
    notify: Notify,
    /// Control events shed under overflow (drop-oldest).
    pub drops: AtomicU64,
}

impl ControlIngressQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a control update. Overflow policy is drop-OLDEST: a stale
    /// queued `ValueChange`/`AppCall` is the least valuable thing here (a
    /// dropped DHT update is re-read on the next watch tick; governance
    /// merges are order-independent), so under pressure we evict the
    /// front and keep the newest. Rate-limited warn + counter surface it.
    pub fn push(&self, update: VeilidUpdate) {
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
    pub async fn pop(&self) -> VeilidUpdate {
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
mod tests {
    use super::*;
    use veilid_core::{VeilidLog, VeilidLogLevel};

    // `Log` is a variant this queue never actually carries (the dispatch
    // loop only pushes `ValueChange`/`AppCall`), but it is the one
    // trivially-constructible `VeilidUpdate` — perfect for exercising the
    // queue's FIFO/overflow mechanics with a distinguishable tag.
    fn update(tag: u64) -> VeilidUpdate {
        VeilidUpdate::Log(Box::new(VeilidLog {
            log_level: VeilidLogLevel::Info,
            message: tag.to_string(),
            backtrace: None,
        }))
    }

    fn tag_of(update: &VeilidUpdate) -> u64 {
        match update {
            VeilidUpdate::Log(log) => log.message.parse().expect("tag parse"),
            _ => panic!("unexpected update variant in test"),
        }
    }

    #[test]
    fn fifo_order_preserved() {
        let q = ControlIngressQueue::new();
        q.push(update(1));
        q.push(update(2));
        q.push(update(3));
        let mut got = Vec::new();
        while q.depth() > 0 {
            got.push(tag_of(&q.inner.lock().pop_front().unwrap()));
        }
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[test]
    fn overflow_drops_oldest_and_counts() {
        let q = ControlIngressQueue::new();
        for i in 0..CONTROL_INGRESS_QUEUE_MAX as u64 {
            q.push(update(i));
        }
        assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
        assert_eq!(q.drops.load(Ordering::Relaxed), 0);

        // One past capacity: the oldest (tag 0) is evicted, the newest
        // lands, depth stays pinned at the ceiling, counter ticks once.
        q.push(update(9999));
        assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
        assert_eq!(q.drops.load(Ordering::Relaxed), 1);
        let tags: Vec<u64> = q.inner.lock().iter().map(tag_of).collect();
        assert_eq!(tags.first(), Some(&1), "oldest element must be dropped");
        assert_eq!(tags.last(), Some(&9999), "newest element must land");
    }

    #[test]
    fn sustained_overflow_keeps_counting() {
        let q = ControlIngressQueue::new();
        for i in 0..(CONTROL_INGRESS_QUEUE_MAX as u64 + 100) {
            q.push(update(i));
        }
        assert_eq!(q.depth(), CONTROL_INGRESS_QUEUE_MAX);
        assert_eq!(q.drops.load(Ordering::Relaxed), 100);
    }

    #[tokio::test]
    async fn pop_wakes_on_push() {
        let q = std::sync::Arc::new(ControlIngressQueue::new());
        let q2 = std::sync::Arc::clone(&q);
        let waiter = tokio::spawn(async move { tag_of(&q2.pop().await) });
        // Give the waiter a moment to park on notified().
        tokio::task::yield_now().await;
        q.push(update(42));
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("pop must wake")
            .expect("task join");
        assert_eq!(got, 42);
    }
}
