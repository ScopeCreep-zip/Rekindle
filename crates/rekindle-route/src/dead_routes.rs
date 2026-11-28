//! Peers' route blobs known dead (plan RH1r-c, RV1g B1–B3): the rule of
//! when a blob is not imported or called again.
//!
//! A blob enters on Veilid's word only, `RouteChange.dead_remote_routes`
//! after a failed remote route test (veilid-core 0.5.8
//! `ping_validator.rs:731-744`, `route_spec_store_cache.rs:397-415`). Call
//! timeouts do not declare a blob dead (B2 owed): a peer that answers no
//! call on a live route (netsim `rejoin_after_kick`, the inviter unanswered
//! during the first join) never republishes it, so no read could revive
//! it. Veilid's import is idempotent
//! (`route_remote.rs:21-28`), so without this rule a caller that re-reads
//! the peer's unchanged record re-imports the same blob and Veilid tests and
//! drops it again, once per attempt. It leaves after [`FAILURE_CACHE`] (the
//! sender rule's failure cache, S4) or when a read of the peer's record
//! returns it at a strictly higher seq than the first read after its death
//! (the peer republished it; [`PeerLiveness::observe`]'s rule).
//!
//! A peer whose route is alive never republishes it (its loopback passes,
//! so Veilid tells it nothing), so a blob whose death was one test ping
//! lost on a slow round trip would stay refused for the whole
//! [`FAILURE_CACHE`]. A read that finds it still published at the same seq
//! one [`REFETCH_GAP`] after its death (or after the last probe) is told to
//! probe it once by a call (plan RH1r-c S2/S3, inventory item 25): the
//! probe alone re-imports it, which Veilid allows and re-tests
//! (`route_remote.rs:21-40`), and an answer clears it ([`DeadRoutes::proven`]).
//!
//! Nothing here releases a route: Veilid already dropped a dead remote
//! route, and a second release is an ERROR (plan C7.9e).
//!
//! [`PeerLiveness::observe`]: crate::liveness::PeerLiveness::observe

use std::collections::HashMap;

use tokio::time::Instant;

use crate::liveness::{FAILURE_CACHE, REFETCH_GAP};

#[derive(Debug, Clone)]
struct Dead {
    at: Instant,
    /// The record seq of the first read after the death; a strictly higher
    /// one clears the entry.
    seen_seq: HashMap<String, u64>,
    /// When it was last handed out for a probe.
    probed: Option<Instant>,
}

/// What a read of a peer's record that returned a blob means for the dead
/// routes ([`DeadRoutes::observe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadRouteVerdict {
    /// The blob is not dead.
    NotDead,
    /// The peer republished it at a higher seq: usable again (B3).
    Cleared,
    /// Still dead, no probe due.
    Held,
    /// Still published at the same seq one [`REFETCH_GAP`] after its death
    /// or its last probe: probe it once now (RH1r-c S2/S3). The probe is
    /// taken: the next read is told again only one gap later.
    Probe,
}

/// The node's dead peer route blobs.
#[derive(Debug, Default)]
pub struct DeadRoutes {
    dead: HashMap<Vec<u8>, Dead>,
}

impl DeadRoutes {
    /// Declare `blob` dead at `now`; `true` when it was not already.
    pub fn declare(&mut self, blob: &[u8], now: Instant) -> bool {
        let fresh = !self.is_dead(blob, now);
        if fresh {
            self.dead.insert(
                blob.to_vec(),
                Dead {
                    at: now,
                    seen_seq: HashMap::new(),
                    probed: None,
                },
            );
        }
        fresh
    }

    /// Whether `blob` is dead at `now` (an entry past [`FAILURE_CACHE`] is
    /// dropped).
    pub fn is_dead(&mut self, blob: &[u8], now: Instant) -> bool {
        match self.dead.get(blob) {
            Some(d) if now.duration_since(d.at) < FAILURE_CACHE => true,
            Some(_) => {
                self.dead.remove(blob);
                false
            }
            None => false,
        }
    }

    /// A call into `blob` was answered (the suspect round's probe): it
    /// delivers, so it leaves. `true` when it was dead.
    pub fn proven(&mut self, blob: &[u8]) -> bool {
        self.dead.remove(blob).is_some()
    }

    /// A read of the peer's record returned `blob` at `seq` at `now`: the
    /// first read after its death is the baseline, a strictly higher seq
    /// clears it, and the baseline seq read again one [`REFETCH_GAP`] after
    /// the death or the last probe asks for a probe ([`DeadRouteVerdict`]).
    pub fn observe(&mut self, blob: &[u8], seq: u64, now: Instant) -> DeadRouteVerdict {
        self.observe_at("", blob, seq, now)
    }

    /// Observe a version scoped to the record that published it.
    pub fn observe_at(
        &mut self,
        record: &str,
        blob: &[u8],
        seq: u64,
        now: Instant,
    ) -> DeadRouteVerdict {
        if !self.is_dead(blob, now) {
            return DeadRouteVerdict::NotDead;
        }
        let Some(d) = self.dead.get_mut(blob) else {
            return DeadRouteVerdict::NotDead;
        };
        let seen = *d.seen_seq.entry(record.to_owned()).or_insert(seq);
        if seq > seen {
            self.dead.remove(blob);
            return DeadRouteVerdict::Cleared;
        }
        let since = d.probed.unwrap_or(d.at);
        if seq == seen && now.duration_since(since) >= REFETCH_GAP {
            d.probed = Some(now);
            DeadRouteVerdict::Probe
        } else {
            DeadRouteVerdict::Held
        }
    }
}

#[cfg(test)]
#[path = "dead_routes_tests.rs"]
mod tests;
