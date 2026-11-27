//! D8's re-fetch cap (threat model D8, plan VU6, decided 2026-11-15).
//!
//! An inspect's network seq is unauthenticated: on a mixed-version network a
//! poisoned 0.5.7 storage node answers an inflated seq with no value behind
//! it. [`RecordPool::read_changed`](crate::RecordPool::read_changed) fetches
//! a subkey from the network when the inspect claims a newer seq, so without
//! a cap every inspect would drive another full fanout for a value that does
//! not exist.
//!
//! So the pool remembers, per (record, subkey), a claimed seq that the
//! forced get did not confirm (it returned nothing newer, or failed). A later
//! claim of the same or a lower seq never re-fetches that subkey; a higher
//! claim re-fetches at most once per backoff window, [`REFETCH_BACKOFF_MIN`]
//! doubling to [`REFETCH_BACKOFF_MAX`]; any get that returns a value at or above the
//! claim clears it. Veilid backs off the same way rather than retrying per
//! hint (its fallback inspection's 30 → 300 s silence timer). The memory
//! lives with the pool's open records: a close or a restart forgets it,
//! costing one fetch.
//!
//! Time is `tokio::time`'s monotonic clock, as the pool's other backoffs
//! read it (the watch re-arm, `watch.rs`; the re-push, `durable.rs`), so the
//! netsim's paused clock drives it.
//!
//! [`RecordPool::read_hinted`](crate::RecordPool::read_hinted) is never
//! capped: a value change obliges the network read, and Veilid's report
//! marks clear only on one (plan VU3). Its gets still clear a claim they
//! confirm.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

/// The first backoff window after a claim goes unconfirmed.
pub(crate) const REFETCH_BACKOFF_MIN: Duration = Duration::from_secs(60);
/// The longest backoff window.
pub(crate) const REFETCH_BACKOFF_MAX: Duration = Duration::from_secs(3600);

/// A claimed seq no get has confirmed.
#[derive(Debug, Clone, Copy)]
struct Claim {
    seq: u32,
    /// The next higher claim may be fetched from then.
    next: Instant,
    backoff: Duration,
}

/// The unconfirmed claims of the pool's open records.
#[derive(Debug, Default)]
pub(crate) struct Unconfirmed {
    claims: HashMap<(String, u32), Claim>,
}

impl Unconfirmed {
    /// Whether an inspect's claim of `claimed` for `key`'s `subkey` may be
    /// fetched from the network at `now`.
    pub(crate) fn may_fetch(&self, key: &str, subkey: u32, claimed: u32, now: Instant) -> bool {
        self.claims
            .get(&(key.to_string(), subkey))
            .is_none_or(|c| claimed > c.seq && now >= c.next)
    }

    /// Any get of `key`'s `subkey` returned a value at `seq`: a claim at or
    /// below it is confirmed.
    pub(crate) fn confirmed(&mut self, key: &str, subkey: u32, seq: u32) {
        let id = (key.to_string(), subkey);
        if self.claims.get(&id).is_some_and(|c| c.seq <= seq) {
            self.claims.remove(&id);
        }
    }

    /// The forced get for a claim of `claimed` answered `got` (the value's
    /// seq; `None` for no value or a failed get) at `now`. Below the claim,
    /// it stays unconfirmed and the window doubles, up to [`REFETCH_BACKOFF_MAX`].
    pub(crate) fn fetched(
        &mut self,
        key: &str,
        subkey: u32,
        claimed: u32,
        got: Option<u32>,
        now: Instant,
    ) {
        if let Some(seq) = got {
            self.confirmed(key, subkey, seq);
            if seq >= claimed {
                return;
            }
        }
        let id = (key.to_string(), subkey);
        let (seq, backoff) = match self.claims.get(&id) {
            Some(c) => (c.seq.max(claimed), (c.backoff * 2).min(REFETCH_BACKOFF_MAX)),
            None => (claimed, REFETCH_BACKOFF_MIN),
        };
        self.claims.insert(
            id,
            Claim {
                seq,
                next: now + backoff,
                backoff,
            },
        );
    }

    /// `key` closed: its claims go with it.
    pub(crate) fn forget(&mut self, key: &str) {
        self.claims.retain(|(k, _), _| k != key);
    }

    /// Every record ended.
    pub(crate) fn clear(&mut self) {
        self.claims.clear();
    }
}

#[cfg(test)]
mod tests;
