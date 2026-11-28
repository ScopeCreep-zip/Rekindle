//! Media-plane liveness ledger — proof-of-life from the CALL
//! transport, not the presence directory.
//!
//! Every production voice stack judges in-call liveness on the call
//! transport itself: Mumble drops a client on UDP/TCP inactivity,
//! Discord's voice gateway on missed heartbeats over the voice
//! connection, WebRTC on ICE consent-freshness / RTP inactivity. The
//! directory (our DHT presence rows) only says who *claims* to be in a
//! channel; whether they are *alive* is what the media plane already
//! knows. A peer whose DHT presence writes fail — routine when the
//! call itself saturates the Veilid relays — goes heartbeat-stale in
//! the directory while their audio keeps arriving.
//!
//! One shared [`MediaLiveness`] per voice session records the last
//! moment each peer proved itself alive on the media plane: an
//! accepted voice packet in the receive loop, or a verified receiver
//! report in the send loop (reports arrive on a fixed ~5 s cadence
//! even from a VAD-silent peer, which is why liveness cannot come from
//! voice packets alone). The presence reconcile consults
//! [`MediaLiveness::live_within`] to veto directory-based eviction and
//! to admit stale-row peers whose media is flowing.
//!
//! Pure and time-injectable like `jitter`: callers pass `now_ms` from
//! one shared millisecond clock (`rekindle_utils::timestamp_ms()` in
//! production — the ledger spans loops with different `Instant`
//! origins, so an origin-relative clock cannot key it). Interior
//! mutability (a `parking_lot::Mutex` held only for sync map ops,
//! never across an `.await` — the guard is !Send) lets one
//! `Arc<MediaLiveness>` be shared by the receive loop, the send loop,
//! and the signaling adapter.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

/// How recently a peer must have proven itself on the media plane to
/// count as live for the roster reconcile. 30 s: long enough to ride
/// out VAD silence gaps between receiver reports (~5 s cadence with
/// slack for a congested return path), short enough that a
/// genuinely-gone peer stops being protected within a scan or two.
pub const MEDIA_LIVE_WINDOW_MS: u64 = 30_000;

/// Entries older than this are pruned on [`MediaLiveness::note`] so
/// the map cannot grow unboundedly across a long session's worth of
/// peers coming and going. Well above [`MEDIA_LIVE_WINDOW_MS`] so
/// pruning can never race a liveness query into a false negative.
const PRUNE_AFTER_MS: u64 = 10 * MEDIA_LIVE_WINDOW_MS;

/// Last media-plane proof-of-life per peer: `pseudonym_hex → last_ms`.
#[derive(Default)]
pub struct MediaLiveness {
    last_seen_ms: Mutex<HashMap<String, u64>>,
}

impl MediaLiveness {
    /// Record media-plane evidence for `pseudonym_hex` at `now_ms`.
    ///
    /// Also prunes entries not refreshed within [`PRUNE_AFTER_MS`] so
    /// the ledger stays bounded by the set of recently-active peers.
    pub fn note(&self, pseudonym_hex: &str, now_ms: u64) {
        let mut map = self.last_seen_ms.lock();
        map.retain(|_, last| now_ms.saturating_sub(*last) <= PRUNE_AFTER_MS);
        map.insert(pseudonym_hex.to_string(), now_ms);
    }

    /// The latest media-plane evidence from any peer, if any: a 1:1
    /// call's consent check (plan CE1, RFC 7675 §5.1), where the one
    /// remote is the peer.
    pub fn latest(&self) -> Option<u64> {
        self.last_seen_ms.lock().values().copied().max()
    }

    /// Peers with media-plane evidence within the last `window_ms`.
    ///
    /// `saturating_sub` makes an entry stamped slightly ahead of
    /// `now_ms` (two callers racing the same wall clock) read as
    /// fresh rather than underflowing into stale.
    pub fn live_within(&self, window_ms: u64, now_ms: u64) -> HashSet<String> {
        let map = self.last_seen_ms.lock();
        map.iter()
            .filter(|(_, last)| now_ms.saturating_sub(**last) <= window_ms)
            .map(|(peer, _)| peer.clone())
            .collect()
    }
}

#[cfg(test)]
#[path = "liveness_tests.rs"]
mod tests;
