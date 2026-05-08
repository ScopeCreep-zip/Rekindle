//! FIFO deduplication cache for gossip fan-out loops.
//!
//! M9.1 — keyed by Blake3-truncated 64-bit hash over
//! `(community_id, sender_pseudonym, dedup_key)` instead of an owned
//! `(String, String, String)` tuple. At ~120 bytes/entry the previous
//! representation grew into MB-scale allocations under churn (10k
//! msgs/community/day across many communities). The compact form is
//! 8 bytes/entry — a 15× footprint reduction with the same FIFO
//! semantics and identical observable behavior.
//!
//! Collision risk at 64 bits is birthday-paradox-bound at ~2^32 entries
//! (~4 billion). The cache's max capacity is bounded at 1024 in
//! production (`AppState::default`), so the practical collision rate is
//! astronomically below 1 in 2^48 — well outside any realistic gossip
//! window. If a collision did occur it would manifest as one extra
//! dropped envelope, which the gossip mesh's three-path delivery is
//! designed to absorb.

use std::collections::{HashSet, VecDeque};

/// Truncated Blake3 hash of a dedup tuple.
type DedupHash = u64;

/// FIFO dedup cache keyed by Blake3-truncated hash of
/// `(community_id, sender_pseudonym, dedup_key)`.
#[derive(Debug, Clone)]
pub struct DedupCache {
    order: VecDeque<DedupHash>,
    entries: HashSet<DedupHash>,
    capacity: usize,
}

impl DedupCache {
    /// Create a new dedup cache with the given maximum number of entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity),
            entries: HashSet::with_capacity(capacity),
            capacity,
        }
    }

    /// Check whether the key has already been seen and insert it if not.
    ///
    /// Returns `true` for duplicates and `false` for newly inserted keys.
    pub fn check_and_insert(&mut self, community_id: &str, sender: &str, dedup_key: &str) -> bool {
        let hash = hash_tuple(community_id, sender, dedup_key);
        if self.entries.contains(&hash) {
            return true;
        }

        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }

        self.order.push_back(hash);
        self.entries.insert(hash);
        false
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    /// Current number of tracked entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Hash the three components into a 64-bit value with explicit length
/// prefixes so two different splits can't alias (e.g. `"abc","",""` vs
/// `"ab","","c"`). Blake3 is keyless here — we want determinism, not
/// MAC properties; collision resistance at 64 bits is the only
/// requirement.
fn hash_tuple(community_id: &str, sender: &str, dedup_key: &str) -> DedupHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(community_id.len() as u64).to_le_bytes());
    hasher.update(community_id.as_bytes());
    hasher.update(&(sender.len() as u64).to_le_bytes());
    hasher.update(sender.as_bytes());
    hasher.update(&(dedup_key.len() as u64).to_le_bytes());
    hasher.update(dedup_key.as_bytes());
    let bytes = hasher.finalize();
    let prefix: [u8; 8] = bytes.as_bytes()[..8]
        .try_into()
        .expect("Blake3 always produces ≥8 bytes");
    u64::from_le_bytes(prefix)
}

#[cfg(test)]
mod tests {
    use super::DedupCache;

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut cache = DedupCache::new(3);
        assert!(!cache.check_and_insert("c", "s", "a"));
        assert!(!cache.check_and_insert("c", "s", "b"));
        assert!(!cache.check_and_insert("c", "s", "c"));
        assert_eq!(cache.len(), 3);

        assert!(!cache.check_and_insert("c", "s", "d"));
        assert_eq!(cache.len(), 3);
        assert!(!cache.check_and_insert("c", "s", "a"));
    }

    #[test]
    fn duplicate_returns_true_without_advancing_order() {
        let mut cache = DedupCache::new(2);
        assert!(!cache.check_and_insert("c", "s", "a"));
        assert!(cache.check_and_insert("c", "s", "a"));
        assert!(!cache.check_and_insert("c", "s", "b"));
        // "a" should still be present (re-insert was a no-op).
        assert!(cache.check_and_insert("c", "s", "a"));
    }

    #[test]
    fn distinct_communities_dont_alias() {
        let mut cache = DedupCache::new(8);
        assert!(!cache.check_and_insert("alpha", "s", "msg1"));
        assert!(!cache.check_and_insert("beta", "s", "msg1"));
        // Same dedup_key in different communities = different entries.
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn distinct_senders_dont_alias() {
        let mut cache = DedupCache::new(8);
        assert!(!cache.check_and_insert("c", "alice", "msg1"));
        assert!(!cache.check_and_insert("c", "bob", "msg1"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn length_prefix_prevents_split_aliasing() {
        // "abc" + "" + "" must hash differently from "ab" + "" + "c".
        // Without the length prefix, both would concatenate to "abc".
        let mut cache = DedupCache::new(8);
        assert!(!cache.check_and_insert("abc", "", ""));
        assert!(!cache.check_and_insert("ab", "", "c"));
        assert_eq!(cache.len(), 2);
    }
}
