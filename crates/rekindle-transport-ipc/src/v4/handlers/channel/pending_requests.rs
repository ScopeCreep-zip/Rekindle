//! PendingRequestTracker — rate-limits concurrent inbound requests.
//!
//! Owned by the Control lane task. Used by:
//! - `datagram/request.rs` — register on inbound request
//! - `channel/ack.rs`, `nack.rs`, `datagram/reply.rs`, `reject.rs` — resolve on response
//! - `heartbeat.rs` — sweep expired entries on tick

use std::collections::HashMap;
use std::time::Instant;

/// A pending datagram request awaiting reply or timeout.
pub struct PendingRequest {
    pub sent_at: Instant,
    pub timeout_ms: u32,
}

impl PendingRequest {
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.sent_at).as_millis() as u32 >= self.timeout_ms
    }
}

/// Tracks in-flight inbound requests for rate limiting and timeout sweep.
/// Bounded at `max_pending` to prevent unbounded memory growth from a
/// malicious peer sending unlimited concurrent requests.
pub struct PendingRequestTracker {
    requests: HashMap<uuid::Uuid, PendingRequest>,
    max_pending: usize,
}

impl PendingRequestTracker {
    pub fn new(max_pending: usize) -> Self {
        Self {
            requests: HashMap::new(),
            max_pending,
        }
    }

    pub fn register(&mut self, message_id: uuid::Uuid, timeout_ms: u32) {
        self.requests.insert(message_id, PendingRequest {
            sent_at: Instant::now(),
            timeout_ms,
        });
    }

    pub fn resolve(&mut self, message_id: &uuid::Uuid) -> bool {
        self.requests.remove(message_id).is_some()
    }

    pub fn count(&self) -> usize {
        self.requests.len()
    }

    pub fn is_full(&self) -> bool {
        self.requests.len() >= self.max_pending
    }

    /// Remove all expired requests. Returns the message IDs of expired
    /// entries so the caller can surface reply-timeout outcomes.
    pub fn sweep_expired(&mut self) -> Vec<uuid::Uuid> {
        let now = Instant::now();
        let expired: Vec<uuid::Uuid> = self.requests.iter()
            .filter(|(_, req)| req.is_expired(now))
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            self.requests.remove(id);
        }
        expired
    }
}
