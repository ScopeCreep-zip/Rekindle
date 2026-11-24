//! D8's re-fetch cap (threat model D8, plan VU6) on the member route
//! re-read (plan E3.4 S4/S5).
//!
//! A send to a member's cached route failed, so the Host forgets the route
//! and re-reads the member's `M`/6 (`reread_route`). The gossip mesh does
//! that after every failed send, and the `ResolveGate` (`rekindle-gossip`) only merges
//! concurrent re-reads; sequential sends to a dead route each paid one
//! `M`/6 fanout for the same value at the same seq. Like the pool's cap on
//! an unconfirmed inspect claim (`rekindle-record-pool/src/refetch.rs`), a
//! re-read that found no usable route is not repeated within a backoff
//! window, [`REREAD_BACKOFF_MIN`] doubling to [`REREAD_BACKOFF_MAX`]: the
//! member's next route reaches us by its ring watch or `HeadAdvanced` hint
//! for subkey 6, and a value above the capped seq (observed by any read)
//! lifts the cap at once. A re-read found no usable route when it read
//! nothing, read the blob that just failed, or read one the transport
//! knows dead (plan RV1g B1: Veilid released it; only a strictly newer seq
//! revives it, B3) — never cached, never sent to.
//!
//! Time is `tokio::time`'s monotonic clock, so the netsim's paused clock
//! drives it.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::time::Instant;

/// The first window after a re-read finds no usable route (D8's
/// `REFETCH_BACKOFF_MIN`).
pub const REREAD_BACKOFF_MIN: Duration = Duration::from_secs(60);
/// The longest window (D8's `REFETCH_BACKOFF_MAX`).
pub const REREAD_BACKOFF_MAX: Duration = Duration::from_secs(3600);

/// A re-read that found no usable route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unproductive {
    /// The seq it read (0 when it read nothing).
    seq: u64,
    /// The next re-read may go to the network from then.
    next: Instant,
    backoff: Duration,
}

/// The capped re-reads of one community, by member pseudonym (hex).
#[derive(Debug, Clone, Default)]
pub(super) struct RereadCap {
    by_member: BTreeMap<String, Unproductive>,
}

impl RereadCap {
    /// Whether a re-read of `member`'s `M`/6 may go to the network at `now`.
    pub(super) fn may_reread(&self, member: &str, now: Instant) -> bool {
        self.by_member.get(member).is_none_or(|u| now >= u.next)
    }

    /// A re-read of `member`'s `M`/6 read `seq` (`None`: nothing) at `now`;
    /// `usable` when it gave a route to send to.
    pub(super) fn reread(&mut self, member: &str, seq: Option<u64>, usable: bool, now: Instant) {
        if usable {
            self.by_member.remove(member);
            return;
        }
        let seq = seq.unwrap_or(0);
        let (seq, backoff) = match self.by_member.get(member) {
            Some(u) => (u.seq.max(seq), (u.backoff * 2).min(REREAD_BACKOFF_MAX)),
            None => (seq, REREAD_BACKOFF_MIN),
        };
        self.by_member.insert(
            member.to_string(),
            Unproductive {
                seq,
                next: now + backoff,
                backoff,
            },
        );
    }

    /// Any read took `member`'s `M`/6 at `seq`: a value above the capped
    /// seq lifts the cap.
    pub(super) fn observed(&mut self, member: &str, seq: u64) {
        if self.by_member.get(member).is_some_and(|u| seq > u.seq) {
            self.by_member.remove(member);
        }
    }

    /// Keep only the members `keep` accepts.
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.by_member.retain(|member, _| keep(member));
    }
}
