//! Inbound `app_call`s, answered concurrently (plan C7.22). Moved from the
//! desktop's `services/veilid/app_call_lane.rs` (plan E4.1 S3): the
//! transport's dispatch loop serves its calls here too.
//!
//! An `app_call` has a deadline that a `ValueChange` does not: veilid-core
//! waits `network.rpc.timeout_ms` (5 s) for the app's answer and then drops
//! the waiter (`rpc_app_call.rs`, `wait_for_op(handle, self.timeout)`), so
//! an answer after that fails with "Unmatched operation id" and the caller
//! sees a timeout. Queued FIFO behind `ValueChange`s — each a DHT read, a
//! DB write and a decrypt — calls missed it: the first C7.20/C7.22 run
//! logged late replies in bursts, and call-media keys never landed.
//!
//! So calls get their own lane, the libp2p request-response shape: each
//! inbound request is served concurrently, at most
//! [`APP_CALL_CONCURRENCY`] at once, and one past that is dropped with a
//! warning (`request-response/src/handler.rs`: "Dropping inbound stream
//! because we are at capacity") — its caller times out and retries, as it
//! would for a lost call. Handlers are never aborted part-way: a reply that
//! misses the deadline is only wasted, while aborting mid-commit is what
//! wedged Veilid's record store (plan C7.6).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use tokio::sync::Semaphore;

/// Most inbound calls served at once. Calls are low-rate (MEK transfers,
/// media keys, bootstrap, attachment chunks); the bound only stops a flood
/// from spawning without limit.
pub const APP_CALL_CONCURRENCY: usize = 64;

/// Bounded concurrent service of inbound `app_call`s.
pub struct AppCallLane {
    slots: Arc<Semaphore>,
    /// Calls dropped because every slot was busy.
    pub drops: AtomicU64,
}

impl Default for AppCallLane {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(APP_CALL_CONCURRENCY)),
            drops: AtomicU64::new(0),
        }
    }
}

impl AppCallLane {
    /// Serve `call` (one inbound `app_call`'s whole handling) on `scope`,
    /// the unlocked session's, or drop it when the lane is full. Both hosts
    /// route their calls here: the transport's dispatch loop and the
    /// desktop's (plan E4.1 S3).
    pub fn dispatch<F>(&self, scope: &Arc<SessionScope>, call: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let Ok(slot) = Arc::clone(&self.slots).try_acquire_owned() else {
            let dropped = self.drops.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(
                dropped_total = dropped,
                "inbound app_call dropped: {APP_CALL_CONCURRENCY} already in service"
            );
            return;
        };
        scope.spawn_or_drop("inbound app_call", async move {
            call.await;
            drop(slot);
        });
    }
}
