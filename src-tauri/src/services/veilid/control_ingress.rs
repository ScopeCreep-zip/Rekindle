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
#[path = "control_ingress_tests.rs"]
mod tests;
