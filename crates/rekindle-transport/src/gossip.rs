//! Gossip mesh primitives — dedup, fanout, Lamport clocks, rate limiting.
//!
//! Per-community peer set management with adaptive fanout degree.
//! Gossip messages are broadcast to a subset of online peers, forwarded
//! with TTL decrement, and deduplicated by content hash.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rekindle_gossip::rate_limit::TokenBucket;

// ── Dedup cache ──────────────────────────────────────────────────────

/// FIFO dedup cache — see [`rekindle_codec::dedup::DedupCache`].
///
/// This crate carried a third copy of this cache that stored the owned
/// `(community_id, sender, dedup_key)` tuple in a `VecDeque` and found
/// duplicates with a linear scan — O(n) per gossip message on the
/// dispatch hot path, allocating three Strings even on a hit. The shared
/// implementation hashes the tuple to 8 bytes and probes a HashSet. The
/// tests below are the transport-side contract and pass unchanged
/// against it.
pub use rekindle_codec::dedup::DedupCache;

// ── Lamport clock ────────────────────────────────────────────────────

/// Lamport logical clock — see [`rekindle_gossip::lamport::LamportClock`].
///
/// This crate carried its own copy whose `merge` was a bare
/// `self.value.max(received) + 1`. That has no drift ceiling and no
/// checked arithmetic, so a single peer sending `lamport = u64::MAX`
/// would overflow the `+ 1` (panic in debug, wrap to 0 in release) and
/// otherwise fast-forward the clock permanently, destroying causal
/// ordering for the rest of the community's lifetime. The shared clock
/// applies the M9.2 drift cap (`MAX_LAMPORT_DRIFT`) and returns
/// `Option<u64>` so the caller drops the offending envelope instead.
pub use rekindle_gossip::lamport::LamportClock;

// ── Fanout degree ────────────────────────────────────────────────────

/// Gossip fan-out degree — see [`rekindle_gossip::mesh::fanout_degree`].
///
/// This crate's copy returned `online_count` (effectively full mesh,
/// D = N-1) for N ≤ 20 where the architecture's epidemic-broadcast
/// parameters specify `min(N, 6)`. Fan-out and TTL are one coupled
/// setting — the 5-hop budget is what lets a sampled D still reach
/// everyone — so a track running uncapped D and a 3-hop TTL was
/// over-sending in small communities and under-reaching in large ones,
/// against peers on the other track that were doing neither.
pub use rekindle_gossip::mesh::fanout_degree;

// ── Rate limiter ─────────────────────────────────────────────────────

/// Soft cap on tracked senders. Above it, idle buckets are pruned.
/// Matches the desktop receiver-side limiter
/// (`src-tauri/services/community/receiver_limits.rs`).
const RATE_LIMIT_SOFT_CAP: usize = 10_000;

/// Buckets untouched for longer than this are dropped when pruning.
const RATE_LIMIT_IDLE_TTL: Duration = Duration::from_secs(60);

/// Receiver-side per-sender gossip rate limiter (architecture §20.2).
///
/// Backed by the shared [`TokenBucket`] at the same 10 msg/s floor the
/// desktop track enforces, so a client that bypasses its own send-side
/// limiter is throttled identically by honest peers on either track —
/// the "reader validates" symmetry the chiral model depends on.
///
/// This replaces a per-sender sliding window of `u64` second-timestamps
/// which, besides being a fourth copy of a gossip primitive, was never
/// consulted: `GossipMesh` constructed one and nothing ever called it,
/// so the daemon had no receiver-side flood protection at all. It also
/// grew a `VecDeque` per sender forever — nothing pruned idle senders.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// sender_key → token bucket.
    buckets: HashMap<String, TokenBucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Check and consume one token for `sender` at `now`.
    ///
    /// Returns `true` if the envelope should be processed, `false` if
    /// the sender is over their floor and it should be dropped.
    /// Time is a parameter so the caller — and the tests — control it.
    pub fn check_at(&mut self, sender: &str, now: Instant) -> bool {
        let accepted = self
            .buckets
            .entry(sender.to_string())
            .or_insert_with(|| TokenBucket::ten_per_second(now))
            .try_consume_at(1, now);
        if self.buckets.len() > RATE_LIMIT_SOFT_CAP {
            self.prune_idle(now);
        }
        accepted
    }

    /// [`Self::check_at`] against the current instant.
    pub fn check(&mut self, sender: &str) -> bool {
        self.check_at(sender, tokio::time::Instant::now().into_std())
    }

    /// Drop buckets idle longer than [`RATE_LIMIT_IDLE_TTL`], so a burst
    /// of one-off senders cannot pin the map indefinitely.
    fn prune_idle(&mut self, now: Instant) {
        self.buckets
            .retain(|_, b| now.saturating_duration_since(b.last_refill()) < RATE_LIMIT_IDLE_TTL);
    }

    /// Remove all tracking for a sender.
    pub fn remove_sender(&mut self, sender: &str) {
        self.buckets.remove(sender);
    }

    /// Clear all tracking.
    pub fn clear(&mut self) {
        self.buckets.clear();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Gossip mesh ──────────────────────────────────────────────────────

/// Per-community gossip overlay state.
///
/// Tracks the members heard from, the Lamport clock and the inbound rate
/// floor. The fan-out targets are not here: the Host picks them from the
/// roster's live beats and their `M`/6 routes (plan E3.4 S5).
pub struct GossipMesh {
    /// Community ID this mesh belongs to.
    pub community_id: String,
    /// All known online members: pseudonym_key → route blob.
    pub online_members: HashMap<String, OnlineMember>,
    /// Lamport clock for outgoing messages.
    pub clock: LamportClock,
    /// Rate limiter for inbound messages.
    pub rate_limiter: RateLimiter,
}

/// Outcome of [`GossipMesh::admit_gossip_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GossipAdmission {
    /// Process the envelope.
    Accept,
    /// Sender is over the per-sender rate floor (§20.2).
    RateLimited,
}

pub use rekindle_types::presence::OnlineMember;

impl GossipMesh {
    pub fn new(community_id: String) -> Self {
        Self {
            community_id,
            online_members: HashMap::new(),
            clock: LamportClock::default(),
            rate_limiter: RateLimiter::default(),
        }
    }

    /// Add or update an online member.
    ///
    /// The `last_seen` timestamp is clamped to the current time to prevent
    /// malicious peers from claiming future timestamps. `now` is the caller's clock
    /// reading in seconds.
    pub fn upsert_member(&mut self, pseudonym: String, mut member: OnlineMember, now: u64) {
        if member.last_seen > now {
            member.last_seen = now;
        }
        self.online_members.insert(pseudonym, member);
    }

    /// Remove a member from the online set and peer set.
    pub fn remove_member(&mut self, pseudonym: &str) {
        self.online_members.remove(pseudonym);
        self.rate_limiter.remove_sender(pseudonym);
    }

    /// Receiver-side admission check for one inbound gossip envelope,
    /// evaluated at `now`.
    ///
    /// Two gates, in order:
    ///
    /// 1. **Rate floor** (§20.2) — the sender's token bucket. Honest
    ///    peers dropping the excess is what makes the floor binding on
    ///    a client that ignores its own send-side limiter.
    /// 2. **Lamport merge** — advances this mesh's clock past the
    ///    received timestamp, by at most `MAX_LAMPORT_DRIFT + 1`
    ///    (`LamportClock::merge` clamps). The envelope is admitted
    ///    whatever it claims; a forged timestamp cannot pin the clock.
    ///
    /// The caller must have verified the envelope signature first —
    /// neither gate is meaningful for an unauthenticated sender.
    pub fn admit_gossip_at(
        &mut self,
        sender: &str,
        lamport_ts: u64,
        now: Instant,
    ) -> GossipAdmission {
        if !self.rate_limiter.check_at(sender, now) {
            return GossipAdmission::RateLimited;
        }
        self.clock.merge(lamport_ts);
        GossipAdmission::Accept
    }

    /// [`Self::admit_gossip_at`] against the current instant.
    pub fn admit_gossip(&mut self, sender: &str, lamport_ts: u64) -> GossipAdmission {
        self.admit_gossip_at(sender, lamport_ts, tokio::time::Instant::now().into_std())
    }

    /// Evict members not seen within `ttl_secs` seconds.
    pub fn evict_stale(&mut self, now_secs: u64, ttl_secs: u64) {
        let cutoff = now_secs.saturating_sub(ttl_secs);
        self.online_members.retain(|_, m| m.last_seen >= cutoff);
    }
}

#[cfg(test)]
#[path = "gossip_tests.rs"]
mod tests;
