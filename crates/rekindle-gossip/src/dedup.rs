//! FIFO deduplication cache for gossip fan-out loops.

use std::collections::{HashSet, VecDeque};

type DedupTuple = (String, String, String);

/// FIFO dedup cache keyed by `(community_id, sender_pseudonym, dedup_key)`.
#[derive(Debug, Clone)]
pub struct DedupCache {
    order: VecDeque<DedupTuple>,
    entries: HashSet<DedupTuple>,
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
        let key = (
            community_id.to_string(),
            sender.to_string(),
            dedup_key.to_string(),
        );
        if self.entries.contains(&key) {
            return true;
        }

        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }

        self.order.push_back(key.clone());
        self.entries.insert(key);
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
}
