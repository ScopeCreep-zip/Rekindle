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
//! drops it again, once per attempt. It leaves after `DEAD_HOLD` (the
//! sender rule's failure cache, S4) or when a read of the peer's record
//! returns it at a strictly higher seq than the first read after its death
//! (the peer republished it; [`PeerLiveness::observe`]'s rule).
//!
//! Nothing here releases a route: Veilid already dropped a dead remote
//! route, and a second release is an ERROR (plan C7.9e).
//!
//! [`PeerLiveness::observe`]: crate::liveness::PeerLiveness::observe

use std::collections::HashMap;

use tokio::time::Instant;

/// DL3 PROBE: Veilid's remote route re-test cooldown, `rpc_timeout × 3`
/// (veilid-core `routing_table/mod.rs:89`), at the 5 s default.
const DEAD_HOLD: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Clone)]
struct Dead {
    at: Instant,
    /// The record seq of the first read after the death; a strictly higher
    /// one clears the entry.
    seen_seq: Option<u64>,
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
                    seen_seq: None,
                },
            );
        }
        fresh
    }

    /// Whether `blob` is dead at `now` (an entry past `DEAD_HOLD` is
    /// dropped).
    pub fn is_dead(&mut self, blob: &[u8], now: Instant) -> bool {
        match self.dead.get(blob) {
            Some(d) if now.duration_since(d.at) < DEAD_HOLD => true,
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

    /// A read of the peer's record returned `blob` at `seq`: the first read
    /// after its death is the baseline, a strictly higher seq clears it.
    /// `true` when this read cleared it.
    pub fn observe(&mut self, blob: &[u8], seq: u64) -> bool {
        let Some(d) = self.dead.get_mut(blob) else {
            return false;
        };
        match d.seen_seq {
            Some(seen) if seq > seen => {
                self.dead.remove(blob);
                true
            }
            Some(_) => false,
            None => {
                d.seen_seq = Some(seq);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_dead_blob_stays_dead_until_the_failure_cache_expires() {
        let t0 = Instant::now();
        let mut dead = DeadRoutes::default();
        assert!(dead.declare(b"A", t0));
        assert!(!dead.declare(b"A", t0), "already dead");
        assert!(dead.is_dead(b"A", t0 + DEAD_HOLD.saturating_sub(Duration::from_secs(1))));
        assert!(!dead.is_dead(b"B", t0));
        assert!(!dead.is_dead(b"A", t0 + DEAD_HOLD), "expired");
        assert!(dead.declare(b"A", t0 + DEAD_HOLD), "declared again");
    }

    #[test]
    fn only_a_strictly_higher_seq_after_the_baseline_clears_a_dead_blob() {
        let t0 = Instant::now();
        let mut dead = DeadRoutes::default();
        dead.declare(b"A", t0);
        assert!(!dead.observe(b"A", 7), "the first read is the baseline");
        assert!(!dead.observe(b"A", 7), "the same record");
        assert!(!dead.observe(b"A", 6), "a lagging replica");
        assert!(dead.is_dead(b"A", t0));
        assert!(dead.observe(b"A", 8), "republished");
        assert!(!dead.is_dead(b"A", t0));
        assert!(!dead.observe(b"B", 9), "never dead");
    }

    #[test]
    fn an_answered_call_proves_a_dead_blob() {
        let t0 = Instant::now();
        let mut dead = DeadRoutes::default();
        dead.declare(b"A", t0);
        assert!(dead.proven(b"A"));
        assert!(!dead.is_dead(b"A", t0));
        assert!(!dead.proven(b"A"), "no longer dead");
    }
}
