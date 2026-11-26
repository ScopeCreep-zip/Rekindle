//! Route evidence source 7 (plan RV2): an unreachable report. Only an
//! outsider sees a General route whose entry leg it cannot reach: the
//! owner's own test is a loopback over flows it opened (veilid-core 0.5.8
//! `route_compile.rs:420-434`, `destination.rs:487-496`), and Veilid tells
//! the importer, never the owner, when its remote test fails
//! (`ping_validator.rs:732-745`). So the caller whose Veilid declared a
//! peer's blob dead, and whose re-read of the peer's profile returned that
//! same blob, reports it once to the owner over the stream the owner
//! already watches from it; the owner rotates on it under the usual
//! evidence bounds. The owner never imports its own blob to test it (VF5:
//! a failed test of a self-import can release our allocated route).

use rekindle_records::port::RouteClass;
use tokio::time::Instant;

use super::Liveness;

/// The BLAKE3 a report names a route blob by.
#[must_use]
pub fn blob_hash(blob: &[u8]) -> [u8; 32] {
    *blake3::hash(blob).as_bytes()
}

impl Liveness {
    /// The hash to report `peer`'s route `blob` by, once: `Some` the first
    /// time a re-read returns this blob while Veilid holds it dead; `None`
    /// when it is not dead or was already reported.
    #[must_use]
    pub fn report_due(&self, peer: &str, blob: &[u8]) -> Option<[u8; 32]> {
        if !self.route_declared_dead(blob) {
            return None;
        }
        let hash = blob_hash(blob);
        self.reported
            .lock()
            .insert((peer.to_owned(), hash))
            .then_some(hash)
    }

    /// `peer`, authenticated by the stream it wrote on and admitted as a
    /// friend or roster member by its ingest, reported the blob with BLAKE3
    /// `reported` unreachable: evidence against our General route when it
    /// names our live blob.
    pub fn route_reported(&self, peer: &str, reported: [u8; 32]) {
        let Some(current) = self.routes.own_blob(RouteClass::General) else {
            return;
        };
        let current = blob_hash(&current);
        tracing::info!(peer = %peer, ours = reported == current, "a peer reported a General route of ours unreachable");
        if let Some(state) = self.general.lock().as_mut() {
            state
                .rules
                .route_reported(peer, reported, current, Instant::now());
        }
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
