//! Peers' route blobs known dead on the transport (plan RV1g B1–B3): the
//! rule of [`rekindle_route::dead_routes`], fed by Veilid's
//! `RouteChange.dead_remote_routes` and by record reads.
//!
//! It lives on the transport, beside the peer registry that hears the same
//! `RouteChange`, not in `rekindle-protocol`'s `RouteImports`: the rule is
//! Veilid-free (ADR 0021's pattern), so both hosts and the netsim run the
//! same code over their route ports, and every import of a peer blob
//! (`TransportNode::import_route`) already passes here.

use rekindle_route::dead_routes::DeadRouteVerdict;
use tokio::time::Instant;

use super::Liveness;
use crate::broadcast::node::TransportNode;
use crate::route_trace;

/// How a one-call probe of a peer's route ended ([`TransportNode::probe_route`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probed {
    /// It answered (a NAK too): it delivers, and it is no longer dead.
    Answered,
    /// The blob did not import.
    ImportFailed(String),
    /// No answer came: it stays as it was.
    Silent(String),
}

impl Liveness {
    /// Veilid declared the routes imported from `blobs` dead (B1). The
    /// routes are not released here or anywhere: Veilid already dropped
    /// them.
    pub fn declare_dead_by_veilid(&self, blobs: &[Vec<u8>]) {
        let now = Instant::now();
        for blob in blobs {
            if self.dead.lock().declare(blob, now) {
                route_trace::declared_dead(blob);
            }
        }
    }

    /// Whether `blob` is a dead peer route: refused at import (B1).
    #[must_use]
    pub fn route_declared_dead(&self, blob: &[u8]) -> bool {
        self.dead.lock().is_dead(blob, Instant::now())
    }

    /// A call into `blob` was answered: it delivers, so it is not dead.
    pub fn route_proven(&self, blob: &[u8]) {
        if self.dead.lock().proven(blob) {
            tracing::info!(blob = %route_trace::blob_tag(blob), "a dead peer route answered: usable again");
        }
    }

    /// A read of the peer's record returned `blob` at DHT `seq` (B3): what
    /// it means for a dead blob, a probe due included (plan RH1r-c S2/S3,
    /// [`DeadRouteVerdict::Probe`]).
    pub fn route_read_at(&self, blob: &[u8], seq: u64) -> DeadRouteVerdict {
        self.route_read_on("", blob, seq)
    }

    /// Observe a sequence on the record that published this route.
    pub fn route_read_on(&self, record: &str, blob: &[u8], seq: u64) -> DeadRouteVerdict {
        let seen = self
            .dead
            .lock()
            .observe_at(record, blob, seq, Instant::now());
        match seen {
            DeadRouteVerdict::Cleared => tracing::info!(
                blob = %route_trace::blob_tag(blob),
                seq,
                "a dead peer route was republished: usable again"
            ),
            DeadRouteVerdict::Probe => tracing::info!(
                blob = %route_trace::blob_tag(blob),
                seq,
                "a dead peer route is still published one re-fetch gap later: probing it"
            ),
            DeadRouteVerdict::NotDead | DeadRouteVerdict::Held => {}
        }
        seen
    }
}

impl TransportNode {
    /// Probe the peer route `blob` once with an `app_call` (plan RH1r-c
    /// S2): the probe alone imports a blob Veilid declared dead (RV1g B1;
    /// every send refuses it, `import_live`), which Veilid allows and tests
    /// again (0.5.8 `route_remote.rs:21-40`). Any answer, a NAK included,
    /// proves it delivers and clears it from the dead routes.
    pub async fn probe_route(&self, blob: &[u8]) -> Probed {
        let live = self.liveness();
        let target = match self.routes().import(blob) {
            Ok(target) => target,
            Err(e) => return Probed::ImportFailed(e.to_string()),
        };
        match self.caller().call_direct(&target, live.peer_probe()).await {
            Ok(_) => {
                live.route_proven(blob);
                Probed::Answered
            }
            Err(e) => Probed::Silent(e.to_string()),
        }
    }
}
