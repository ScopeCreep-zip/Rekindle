//! Peers' route blobs known dead on the transport (plan RV1g B1–B3): the
//! rule of [`rekindle_route::dead_routes`], fed by Veilid's
//! `RouteChange.dead_remote_routes`, by call timeouts and by record reads.
//!
//! It lives on the transport, beside the peer registry that hears the same
//! `RouteChange`, not in `rekindle-protocol`'s `RouteImports`: the rule is
//! Veilid-free (ADR 0021's pattern), so both hosts and the netsim run the
//! same code over their route ports, and every import of a peer blob
//! (`TransportNode::import_route`) already passes here.

use rekindle_route::dead_routes::DeadBy;
use tokio::time::Instant;

use super::Liveness;
use crate::route_trace;

impl Liveness {
    /// Veilid declared the routes imported from `blobs` dead (B1). The
    /// routes are not released here or anywhere: Veilid already dropped
    /// them.
    pub fn declare_dead_by_veilid(&self, blobs: &[Vec<u8>]) {
        let now = Instant::now();
        for blob in blobs {
            if self.dead.lock().declare(blob, now) {
                route_trace::declared_dead(blob, DeadBy::Veilid);
            }
        }
    }

    /// Whether `blob` is a dead peer route: refused at import (B1).
    #[must_use]
    pub fn route_declared_dead(&self, blob: &[u8]) -> bool {
        self.dead.lock().is_dead(blob, Instant::now())
    }

    /// A call into `blob` timed out (B2).
    pub fn route_call_timed_out(&self, blob: &[u8]) {
        if self.dead.lock().timed_out(blob, Instant::now()) {
            route_trace::declared_dead(blob, DeadBy::Timeouts);
        }
    }

    /// A call into `blob` was answered (B2).
    pub fn route_call_answered(&self, blob: &[u8]) {
        self.dead.lock().answered(blob);
    }

    /// A read of the peer's record returned `blob` at DHT `seq` (B3).
    pub fn route_read_at(&self, blob: &[u8], seq: u64) {
        if self.dead.lock().observe(blob, seq) {
            tracing::info!(
                blob = %route_trace::blob_tag(blob),
                seq,
                "a dead peer route was republished: usable again"
            );
        }
    }
}
