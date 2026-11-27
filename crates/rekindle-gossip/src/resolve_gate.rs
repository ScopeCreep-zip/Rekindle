//! Single-flight gate for per-peer DHT route re-resolution.
//!
//! When a peer's cached route goes stale, every in-flight envelope to
//! that peer fails and wants to re-resolve from the SMPL presence
//! registry. Without coalescing, a queued-broadcast drain fires one
//! DHT resolution per envelope for the same peer in the same instant
//! (observed: 23 concurrent resolves on one presence-poll drain). The
//! gate lets exactly one task lead the resolution per
//! `(community, peer)`; the rest wait for the leader to finish and
//! retry against the route the leader wrote back to the overlay.

use std::collections::HashMap;
use std::sync::Arc;

/// Per-`(community, peer)` single-flight lock set. One entry per peer
/// ever resolved this session — bounded by community size, never
/// trimmed (re-resolution recurs for a session-long roster). The map
/// guard is parking_lot and is never held across an await.
#[derive(Default)]
pub struct ResolveGate {
    inner: parking_lot::Mutex<HashMap<(String, String), Arc<tokio::sync::Mutex<()>>>>,
}

impl ResolveGate {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&self, community_id: &str, peer: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.inner.lock();
        Arc::clone(
            map.entry((community_id.to_string(), peer.to_string()))
                .or_default(),
        )
    }

    /// Become the resolution leader for this peer, or `None` when a
    /// resolution is already in flight. The returned guard must be
    /// held across the resolve + write-back; dropping it releases the
    /// waiters.
    pub fn try_lead(
        &self,
        community_id: &str,
        peer: &str,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.slot(community_id, peer).try_lock_owned().ok()
    }

    /// Wait until the in-flight resolution for this peer completes.
    /// Returns immediately when none is in flight.
    pub async fn wait(&self, community_id: &str, peer: &str) {
        let slot = self.slot(community_id, peer);
        drop(slot.lock_owned().await);
    }
}

#[cfg(test)]
#[path = "resolve_gate_tests.rs"]
mod tests;
