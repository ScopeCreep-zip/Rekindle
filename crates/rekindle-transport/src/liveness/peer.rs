//! The sender side (plan RH1r-c S1–S6): a peer's route is suspected from
//! missing answers, re-read from their profile at most once per 60 s, and
//! probed when the record is the one we hold; after six rounds the peer is
//! shown unreachable and the fast path backs off while the durable chain
//! carries the traffic. Any authenticated frame from the peer resets it.

use rekindle_route::liveness::{Fetched, PeerLiveness, Suspicion};
use tokio::time::Instant;

use super::Liveness;

impl Liveness {
    /// A send to `peer` that expects an answer went out (S1).
    pub fn sent(&self, peer: &str) {
        self.peers
            .lock()
            .entry(peer.to_string())
            .or_default()
            .sent(Instant::now());
    }

    /// An authenticated frame from `peer` arrived (S5); `true` when the
    /// peer was shown unreachable until now.
    pub fn heard(&self, peer: &str) -> bool {
        self.peers
            .lock()
            .get_mut(peer)
            .is_some_and(PeerLiveness::heard)
    }

    /// Whether a suspect round is due for `peer` now (S2, S3).
    #[must_use]
    pub fn round_due(&self, peer: &str) -> bool {
        self.peers
            .lock()
            .get(peer)
            .is_some_and(|p| p.before_send(Instant::now()) == Suspicion::Refetch)
    }

    /// Begin a suspect round for `peer` against the route `blob` we held;
    /// `true` when the peer now shows as unreachable (S4).
    pub fn begin_round(&self, peer: &str, blob: Option<Vec<u8>>) -> bool {
        self.peers
            .lock()
            .entry(peer.to_string())
            .or_default()
            .begin_round(Instant::now(), blob)
    }

    /// A read of `peer`'s route record found `seq`.
    pub fn observe(&self, peer: &str, seq: u32) -> Fetched {
        self.peers
            .lock()
            .entry(peer.to_string())
            .or_default()
            .observe(seq)
    }

    /// Whether `blob` is `peer`'s suspected route, not to be used yet.
    #[must_use]
    pub fn failure_cached(&self, peer: &str, blob: &[u8]) -> bool {
        self.peers
            .lock()
            .get(peer)
            .is_some_and(|p| p.failure_cached(Instant::now(), blob))
    }

    /// A probe of `peer`'s suspected route answered.
    pub fn probe_answered(&self, peer: &str) -> bool {
        let mut peers = self.peers.lock();
        let Some(state) = peers.get_mut(peer) else {
            return false;
        };
        state.clear_failure();
        state.heard()
    }

    /// Whether the fast path to `peer` may be tried now (S4).
    pub fn fast_path_allowed(&self, peer: &str) -> bool {
        self.peers
            .lock()
            .get_mut(peer)
            .is_none_or(|p| p.fast_path_allowed(Instant::now()))
    }

    /// A liveness probe datagram for a peer's route: its answer, a NAK
    /// included, proves the route delivers (S2).
    #[must_use]
    pub fn peer_probe(&self) -> Vec<u8> {
        self.probe().1
    }
}
