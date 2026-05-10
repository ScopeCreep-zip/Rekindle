//! Gossip mesh primitives — dedup, fanout, Lamport clocks, rate limiting.
//!
//! Per-community peer set management with adaptive fanout degree.
//! Gossip messages are broadcast to a subset of online peers, forwarded
//! with TTL decrement, and deduplicated by content hash.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

// ── Dedup cache ──────────────────────────────────────────────────────

/// Content-hash dedup cache with O(1) lookup via Blake3-derived 64-bit keys.
///
/// Each entry is a 64-bit hash of `(community_id, sender, dedup_key)`.
/// Capacity-bounded with FIFO eviction. Memory per entry: 8 bytes
/// (vs ~120 bytes for the previous triple-String VecDeque).
pub struct DedupCache {
    /// Blake3-derived 64-bit keys for O(1) membership check.
    seen: HashSet<u64>,
    /// FIFO ring for capacity eviction — tracks insertion order.
    ring: VecDeque<u64>,
    capacity: usize,
}

impl DedupCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: HashSet::with_capacity(capacity),
            ring: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Check if this message has been seen before. If not, insert it.
    ///
    /// Returns `true` if the message is a **duplicate** (already seen).
    /// Returns `false` if the message is **new** (just inserted).
    pub fn check_and_insert(
        &mut self,
        community_id: &str,
        sender: &str,
        dedup_key: &str,
    ) -> bool {
        let key = Self::hash_key(community_id, sender, dedup_key);

        if self.seen.contains(&key) {
            return true;
        }

        if self.seen.len() >= self.capacity {
            if let Some(old) = self.ring.pop_front() {
                self.seen.remove(&old);
            }
        }
        self.seen.insert(key);
        self.ring.push_back(key);
        false
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.seen.clear();
        self.ring.clear();
    }

    /// Compute a 64-bit hash key from the dedup triple.
    ///
    /// Uses Blake3 truncated to 64 bits. Birthday-paradox collision probability
    /// at capacity 2048: ~2^-52 (negligible). A collision silently drops one
    /// legitimate message — acceptable tradeoff for O(1) lookup and 8-byte entries.
    fn hash_key(community_id: &str, sender: &str, dedup_key: &str) -> u64 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(community_id.as_bytes());
        hasher.update(b"|");
        hasher.update(sender.as_bytes());
        hasher.update(b"|");
        hasher.update(dedup_key.as_bytes());
        let hash = hasher.finalize();
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("blake3 produces 32 bytes"))
    }
}

// ── Lamport clock ────────────────────────────────────────────────────

/// Lamport logical clock for causal message ordering.
///
/// Increment on every local send. Merge with `max(local, received) + 1`
/// on every receive. This ensures a total order consistent with causality.
#[derive(Debug, Clone, Copy, Default)]
pub struct LamportClock {
    value: u64,
}

impl LamportClock {
    pub fn new(initial: u64) -> Self {
        Self { value: initial }
    }

    /// Increment for a local event (message send). Returns the new value.
    pub fn increment(&mut self) -> u64 {
        self.value += 1;
        self.value
    }

    /// Merge with a received timestamp. Returns the new local value.
    pub fn merge(&mut self, received: u64) -> u64 {
        self.value = self.value.max(received) + 1;
        self.value
    }

    /// Current clock value.
    pub fn value(self) -> u64 {
        self.value
    }
}

// ── Fanout degree ────────────────────────────────────────────────────

/// Compute the gossip fanout degree based on community size.
///
/// - 1-20 members: full mesh (D = N-1, capped at total)
/// - 21-60 members: D = 6
/// - 61+ members: D = 8
pub fn fanout_degree(online_count: usize) -> usize {
    match online_count {
        0 => 0,
        1..=20 => online_count,
        21..=60 => 6,
        _ => 8,
    }
}

// ── Rate limiter ─────────────────────────────────────────────────────

/// Per-sender rate limiter tracking last send timestamps.
///
/// Prevents any single sender from flooding the gossip mesh.
/// Default: max 10 messages per second per sender.
pub struct RateLimiter {
    /// sender_key → list of send timestamps (seconds since epoch).
    windows: HashMap<String, VecDeque<u64>>,
    /// Maximum messages per window.
    max_per_window: usize,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    pub fn new(max_per_window: usize, window_secs: u64) -> Self {
        Self {
            windows: HashMap::new(),
            max_per_window,
            window_secs,
        }
    }

    /// Check if a sender is within their rate limit. If allowed, records
    /// the event and returns `true`. If rate-limited, returns `false`.
    pub fn check_and_record(&mut self, sender: &str, now_secs: u64) -> bool {
        let window = self
            .windows
            .entry(sender.to_string())
            .or_default();

        // Evict entries outside the window
        let cutoff = now_secs.saturating_sub(self.window_secs);
        while window.front().is_some_and(|&ts| ts < cutoff) {
            window.pop_front();
        }

        if window.len() >= self.max_per_window {
            return false;
        }

        window.push_back(now_secs);
        true
    }

    /// Remove all tracking for a sender.
    pub fn remove_sender(&mut self, sender: &str) {
        self.windows.remove(sender);
    }

    /// Clear all tracking.
    pub fn clear(&mut self) {
        self.windows.clear();
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(10, 1)
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

/// An online community member with their route data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineMember {
    /// Veilid private route blob for reaching this member.
    pub route_blob: Vec<u8>,
    /// Last advertised status string.
    pub status: String,
    /// Timestamp (seconds since epoch) of last valid contact.
    pub last_seen: u64,
}

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
        assert_eq!(clock.merge(10), 11);
        assert_eq!(clock.merge(5), 12); // max(12, 5) + 1
    }

    #[test]
    fn fanout_degree_thresholds() {
        assert_eq!(fanout_degree(0), 0);
        assert_eq!(fanout_degree(1), 1);
        assert_eq!(fanout_degree(20), 20);
        assert_eq!(fanout_degree(21), 6);
        assert_eq!(fanout_degree(60), 6);
        assert_eq!(fanout_degree(61), 8);
        assert_eq!(fanout_degree(1000), 8);
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut rl = RateLimiter::new(3, 1);
        assert!(rl.check_and_record("s1", 100));
        assert!(rl.check_and_record("s1", 100));
        assert!(rl.check_and_record("s1", 100));
        assert!(!rl.check_and_record("s1", 100)); // 4th in same second
    }

    #[test]
    fn rate_limiter_resets_after_window() {
        let mut rl = RateLimiter::new(2, 1);
        assert!(rl.check_and_record("s1", 100));
        assert!(rl.check_and_record("s1", 100));
        assert!(!rl.check_and_record("s1", 100));
        assert!(rl.check_and_record("s1", 102)); // window expired
    }
}
