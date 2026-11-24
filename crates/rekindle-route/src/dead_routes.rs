//! Peers' route blobs known dead (plan RH1r-c, RV1g B1–B3): the rule of
//! when a blob is not imported or called again.
//!
//! A blob enters on Veilid's word, `RouteChange.dead_remote_routes` after a
//! failed remote route test (veilid-core 0.5.8 `ping_validator.rs:731-744`,
//! `route_spec_store_cache.rs:397-415`), or after two consecutive calls into
//! it timed out with no answer between (a call into a released route ends
//! as a timeout in 0.5.8, plan E3.4 S4). Veilid's import is idempotent
//! (`route_remote.rs:21-28`), so without this rule a caller that re-reads
//! the peer's unchanged record re-imports the same blob and Veilid tests and
//! drops it again, once per attempt. It leaves after [`FAILURE_CACHE`] (the
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

use crate::liveness::FAILURE_CACHE;

/// Consecutive timed-out calls into one blob that declare it dead (B2).
pub const DEAD_AFTER_TIMEOUTS: u32 = 2;

/// Why a blob was declared dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadBy {
    /// Veilid's `RouteChange.dead_remote_routes`.
    Veilid,
    /// [`DEAD_AFTER_TIMEOUTS`] consecutive timed-out calls.
    Timeouts,
}

#[derive(Debug, Clone)]
struct Dead {
    at: Instant,
    /// The record seq of the first read after the death; a strictly higher
    /// one clears the entry.
    seen_seq: Option<u64>,
}

/// The node's dead peer route blobs and the timeout streaks toward one.
#[derive(Debug, Default)]
pub struct DeadRoutes {
    dead: HashMap<Vec<u8>, Dead>,
    timeouts: HashMap<Vec<u8>, u32>,
}

impl DeadRoutes {
    /// Declare `blob` dead at `now`; `true` when it was not already.
    pub fn declare(&mut self, blob: &[u8], now: Instant) -> bool {
        self.timeouts.remove(blob);
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

    /// A call into `blob` timed out at `now`; `true` when this timeout
    /// declared it dead (B2).
    pub fn timed_out(&mut self, blob: &[u8], now: Instant) -> bool {
        let streak = self.timeouts.entry(blob.to_vec()).or_insert(0);
        *streak += 1;
        if *streak >= DEAD_AFTER_TIMEOUTS {
            return self.declare(blob, now);
        }
        false
    }

    /// A call into `blob` was answered: its timeout streak ends.
    pub fn answered(&mut self, blob: &[u8]) {
        self.timeouts.remove(blob);
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
        assert!(dead.is_dead(
            b"A",
            t0 + FAILURE_CACHE.saturating_sub(Duration::from_secs(1))
        ));
        assert!(!dead.is_dead(b"B", t0));
        assert!(!dead.is_dead(b"A", t0 + FAILURE_CACHE), "expired");
        assert!(dead.declare(b"A", t0 + FAILURE_CACHE), "declared again");
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
    fn two_consecutive_timeouts_declare_a_blob_dead_and_an_answer_resets() {
        let t0 = Instant::now();
        let mut dead = DeadRoutes::default();
        assert!(!dead.timed_out(b"A", t0));
        dead.answered(b"A");
        assert!(!dead.timed_out(b"A", t0), "the answer reset the streak");
        assert!(!dead.is_dead(b"A", t0));
        assert!(dead.timed_out(b"A", t0), "the second in a row");
        assert!(dead.is_dead(b"A", t0));
        assert!(!dead.timed_out(b"B", t0), "streaks are per blob");
    }
}
