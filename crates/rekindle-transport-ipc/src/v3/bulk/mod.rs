pub mod counters;
pub mod pool;
pub mod send;
pub mod recv;

use std::sync::Arc;

use rekindle_transport_buff::DispatchQueue;
use rekindle_transport_buff::adapters::tokio::TokioWake;

use crate::v3::audit::chain::LinkInput;

/// Lock-free audit link queue shared between rayon workers (push) and the
/// control loop (pop via TokioWake notification). Replaces tokio mpsc for
/// the audit path — eliminates channel backpressure at 200K connections.
pub type AuditQueue = Arc<DispatchQueue<LinkInput, TokioWake>>;

/// Create an audit queue and its TokioWake handle.
/// Capacity should be >= inflight_sem permits (rayon_workers × 2) so
/// try_push is structurally infallible under normal operation.
pub fn new_audit_queue(capacity: usize) -> (AuditQueue, TokioWake) {
    let wake = TokioWake::new();
    let queue = Arc::new(DispatchQueue::new(capacity, wake.clone()));
    (queue, wake)
}
