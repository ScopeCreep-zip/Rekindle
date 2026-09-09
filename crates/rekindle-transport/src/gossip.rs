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
            .or_insert_with(TokenBucket::ten_per_second)
            .try_consume_at(1, now);
        if self.buckets.len() > RATE_LIMIT_SOFT_CAP {
            self.prune_idle(now);
        }
        accepted
    }

    /// [`Self::check_at`] against the current instant.
    pub fn check(&mut self, sender: &str) -> bool {
        self.check_at(sender, Instant::now())
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
/// Tracks online members, selected gossip peers, and provides methods
/// for peer selection and broadcast target computation.
pub struct GossipMesh {
    /// Community ID this mesh belongs to.
    pub community_id: String,
    /// All known online members: pseudonym_key → route blob.
    pub online_members: HashMap<String, OnlineMember>,
    /// Selected gossip peers (subset of online_members, size = fanout degree).
    pub peers: HashMap<String, OnlineMember>,
    /// Lamport clock for outgoing messages.
    pub clock: LamportClock,
    /// Rate limiter for inbound messages.
    pub rate_limiter: RateLimiter,
}

/// Outcome of [`GossipMesh::admit_gossip_at`].
///
/// Distinguishes the two rejection reasons rather than returning a
/// bool: they mean different things operationally — one is a noisy
/// peer, the other is a peer sending forged-future timestamps — and the
/// dispatch layer logs them separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GossipAdmission {
    /// Process the envelope.
    Accept,
    /// Sender is over the per-sender rate floor (§20.2).
    RateLimited,
    /// Lamport timestamp more than `MAX_LAMPORT_DRIFT` ahead of ours.
    LamportDrift,
}

pub use rekindle_types::presence::OnlineMember;

impl GossipMesh {
    pub fn new(community_id: String) -> Self {
        Self {
            community_id,
            online_members: HashMap::new(),
            peers: HashMap::new(),
            clock: LamportClock::default(),
            rate_limiter: RateLimiter::default(),
        }
    }

    /// Update the peer set from the current online members.
    ///
    /// Selects up to `fanout_degree(online_count)` peers, preferring
    /// those with the most recent `last_seen` timestamps.
    pub fn refresh_peer_set(&mut self, my_pseudonym: &str) {
        let degree = fanout_degree(self.online_members.len());

        let mut candidates: Vec<(&String, &OnlineMember)> = self
            .online_members
            .iter()
            .filter(|(key, _)| *key != my_pseudonym)
            .collect();

        // Sort by last_seen descending (most recently seen first)
        candidates.sort_by(|a, b| b.1.last_seen.cmp(&a.1.last_seen));
        candidates.truncate(degree);

        self.peers.clear();
        for (key, member) in candidates {
            self.peers.insert(key.clone(), member.clone());
        }
    }

    /// Add or update an online member.
    ///
    /// The `last_seen` timestamp is clamped to the current time to prevent
    /// malicious peers from claiming future timestamps to dominate peer
    /// selection in `refresh_peer_set`.
    pub fn upsert_member(&mut self, pseudonym: String, mut member: OnlineMember) {
        let now = rekindle_utils::timestamp_secs();
        if member.last_seen > now {
            member.last_seen = now;
        }
        self.online_members.insert(pseudonym, member);
    }

    /// Remove a member from the online set and peer set.
    pub fn remove_member(&mut self, pseudonym: &str) {
        self.online_members.remove(pseudonym);
        self.peers.remove(pseudonym);
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
    /// 2. **Lamport merge** — advances this mesh's clock to
    ///    `max(local, received) + 1`, or rejects a timestamp more than
    ///    `MAX_LAMPORT_DRIFT` ahead without advancing.
    ///
    /// A drift-rejected envelope has already spent a token. That is
    /// deliberate: forged timestamps are sender misbehaviour and should
    /// draw down the same budget as any other traffic.
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
        if self.clock.merge(lamport_ts).is_none() {
            return GossipAdmission::LamportDrift;
        }
        GossipAdmission::Accept
    }

    /// [`Self::admit_gossip_at`] against the current instant.
    pub fn admit_gossip(&mut self, sender: &str, lamport_ts: u64) -> GossipAdmission {
        self.admit_gossip_at(sender, lamport_ts, Instant::now())
    }

    /// Evict members not seen within `ttl_secs` seconds.
    pub fn evict_stale(&mut self, now_secs: u64, ttl_secs: u64) {
        let cutoff = now_secs.saturating_sub(ttl_secs);
        self.online_members.retain(|_, m| m.last_seen >= cutoff);
        self.peers.retain(|_, m| m.last_seen >= cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_detects_duplicates() {
        let mut cache = DedupCache::new(10);
        assert!(!cache.check_and_insert("c1", "s1", "msg1"));
        assert!(cache.check_and_insert("c1", "s1", "msg1"));
        assert!(!cache.check_and_insert("c1", "s1", "msg2"));
    }

    #[test]
    fn dedup_evicts_oldest_at_capacity() {
        let mut cache = DedupCache::new(2);
        assert!(!cache.check_and_insert("c1", "s1", "a"));
        assert!(!cache.check_and_insert("c1", "s1", "b"));
        assert!(!cache.check_and_insert("c1", "s1", "c")); // evicts "a"
        assert!(!cache.check_and_insert("c1", "s1", "a")); // "a" was evicted, no longer a dup
    }

    #[test]
    fn lamport_clock_increment_and_merge() {
        let mut clock = LamportClock::new(0);
        assert_eq!(clock.increment(), 1);
        assert_eq!(clock.increment(), 2);
        // `merge` now reports acceptance: the shared clock refuses a
        // received value more than MAX_LAMPORT_DRIFT ahead instead of
        // adopting it, so the caller can drop that envelope.
        assert_eq!(clock.merge(10), Some(11));
        assert_eq!(clock.merge(5), Some(12)); // max(12, 5) + 1
    }

    #[test]
    fn lamport_clock_rejects_forged_future_timestamp() {
        // The copy this crate used to carry computed `max(v, recv) + 1`
        // unchecked, so `u64::MAX` here overflowed the `+ 1` outright.
        let mut clock = LamportClock::new(2);
        assert_eq!(clock.merge(u64::MAX), None);
        assert_eq!(clock.current(), 2, "rejected merge must not advance");
    }

    #[test]
    fn fanout_degree_thresholds() {
        assert_eq!(fanout_degree(0), 0);
        assert_eq!(fanout_degree(1), 1);
        // N ≤ 20 is min(N, 6), not N: the architecture's epidemic
        // parameters cap fan-out at 6 and rely on the 5-hop TTL to
        // finish the job. This crate previously asserted 20 here,
        // pinning its own deviation.
        assert_eq!(fanout_degree(7), 6);
        assert_eq!(fanout_degree(20), 6);
        assert_eq!(fanout_degree(21), 6);
        assert_eq!(fanout_degree(60), 6);
        assert_eq!(fanout_degree(61), 8);
        assert_eq!(fanout_degree(1000), 8);
    }

    // ── Cross-track parity ───────────────────────────────────────
    //
    // Fan-out degree and TTL are *protocol* parameters, not local
    // tuning: they set how far a message travels through a mesh whose
    // members run both this crate (daemon) and rekindle-gossip
    // (desktop). Architecture "Epidemic broadcast parameters" fixes
    // them together — D = min(N,6) for N ≤ 20, 6 for 21–60, 8 for 61+,
    // TTL 5 — and rekindle-gossip's own note explains the coupling:
    // "the dedup cache + 5-hop TTL guarantee delivery without flooding
    // even when most peers don't receive a direct copy". Tuning one
    // without the other silently changes coverage, so these assert
    // against the shared implementations rather than against literals.

    #[test]
    fn fanout_degree_matches_the_shared_gossip_primitive() {
        for online in [0usize, 1, 5, 6, 7, 19, 20, 21, 60, 61, 1000] {
            assert_eq!(
                fanout_degree(online),
                rekindle_gossip::mesh::fanout_degree(online),
                "fan-out degree diverged from rekindle-gossip at N={online}"
            );
        }
    }

    #[test]
    fn initial_gossip_ttl_is_the_shared_default() {
        // There is only one TTL constant now. This crate used to carry a
        // second (`broadcast::gossip::DEFAULT_TTL`, briefly 3 against
        // codec's 5) and this test pinned them together; the whole
        // postcard gossip stack it belonged to is gone, so the
        // assertion is that the envelope every track signs starts at the
        // architecture's five hops.
        assert_eq!(
            rekindle_codec::envelope::DEFAULT_TTL,
            5,
            "the mesh is specified at a 5-hop TTL"
        );
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        // 10 msg/s floor: the 11th in the same instant is dropped.
        let mut rl = RateLimiter::new();
        let t0 = Instant::now();
        for i in 0..10 {
            assert!(rl.check_at("s1", t0), "message {i} within floor");
        }
        assert!(!rl.check_at("s1", t0), "11th in same instant is over floor");
    }

    #[test]
    fn rate_limiter_resets_after_window() {
        let mut rl = RateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(rl.check_at("s1", t0));
        }
        assert!(!rl.check_at("s1", t0));
        // Refill is continuous, not a window reset: a second later the
        // bucket is back to full capacity.
        assert!(rl.check_at("s1", t0 + Duration::from_secs(1)));
    }

    // ── Receiver-side admission ──────────────────────────────────

    #[test]
    fn admit_gossip_accepts_normal_traffic_and_advances_the_clock() {
        let mut mesh = GossipMesh::new("c1".to_string());
        let t0 = Instant::now();
        assert_eq!(
            mesh.admit_gossip_at("alice", 1, t0),
            GossipAdmission::Accept
        );
        // The clock must actually move — the whole point of merging is
        // that our next send is ordered after what we just received.
        assert_eq!(mesh.clock.current(), 2, "merge(1) on a 0 clock → 2");
        assert_eq!(
            mesh.admit_gossip_at("alice", 9, t0),
            GossipAdmission::Accept
        );
        assert_eq!(mesh.clock.current(), 10);
    }

    #[test]
    fn admit_gossip_rate_limits_a_flooding_sender() {
        let mut mesh = GossipMesh::new("c1".to_string());
        let t0 = Instant::now();
        for i in 0..10 {
            assert_eq!(
                mesh.admit_gossip_at("flooder", i + 1, t0),
                GossipAdmission::Accept
            );
        }
        assert_eq!(
            mesh.admit_gossip_at("flooder", 11, t0),
            GossipAdmission::RateLimited
        );
        // A different sender still gets through — the floor is
        // per-sender, so one flooder cannot mute the community.
        assert_eq!(mesh.admit_gossip_at("bob", 11, t0), GossipAdmission::Accept);
    }

    #[test]
    fn admit_gossip_rejects_forged_future_lamport_without_advancing() {
        let mut mesh = GossipMesh::new("c1".to_string());
        let t0 = Instant::now();
        assert_eq!(
            mesh.admit_gossip_at("alice", 5, t0),
            GossipAdmission::Accept
        );
        let before = mesh.clock.current();

        assert_eq!(
            mesh.admit_gossip_at("mallory", u64::MAX, t0),
            GossipAdmission::LamportDrift
        );
        assert_eq!(
            mesh.clock.current(),
            before,
            "a rejected envelope must not move the clock — otherwise one \
             forged message permanently breaks ordering for the community"
        );

        // And the mesh keeps working afterwards.
        assert_eq!(
            mesh.admit_gossip_at("alice", before, t0),
            GossipAdmission::Accept
        );
    }

    #[test]
    fn rate_limiter_meters_each_sender_separately() {
        // One flooding sender must not consume another's budget.
        let mut rl = RateLimiter::new();
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(rl.check_at("flooder", t0));
        }
        assert!(!rl.check_at("flooder", t0));
        assert!(rl.check_at("quiet", t0), "unrelated sender is unaffected");
    }
}
