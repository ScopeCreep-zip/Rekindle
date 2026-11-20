//! Gossip message deduplication cache and key extraction.
//!
//! Prevents infinite gossip forwarding loops and duplicate processing.
//! FIFO eviction when capacity is exceeded. Every envelope is keyed by
//! its sender-chosen message id ([`message_dedup_key`]): epidemic
//! broadcast suppresses copies of one message, never a deliberate
//! re-send of the same content.

use std::collections::{HashSet, VecDeque};

use crate::envelope::SignedEnvelope;

/// Truncated Blake3 hash of a `(community_id, sender, dedup_key)` tuple.
type DedupHash = u64;

/// FIFO dedup cache — returns true if a message was already seen.
///
/// Keyed by a Blake3-truncated 64-bit hash of
/// `(community_id, sender_pseudonym, dedup_key)` rather than the owned
/// tuple. At ~120 bytes/entry the tuple form grew into MB-scale
/// allocations under churn (10k msgs/community/day across many
/// communities); the hash is 8 bytes/entry — a ~15x footprint reduction
/// with identical observable behaviour — and turns the lookup from a
/// scan into a set probe.
///
/// Collision risk at 64 bits is birthday-bound at ~2^32 entries.
/// Production capacity is 1024, so the practical rate is far below
/// 1 in 2^48. A collision would drop one extra envelope, which the
/// three-path delivery model already absorbs.
///
/// This design came from `rekindle-gossip`, which forked this module
/// rather than reusing it and then optimised its copy;
/// `rekindle-transport` had a third copy that kept the tuple form and
/// scanned it linearly. Both now re-export this one.
#[derive(Debug, Clone)]
pub struct DedupCache {
    order: VecDeque<DedupHash>,
    entries: HashSet<DedupHash>,
    capacity: usize,
}

impl DedupCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity),
            entries: HashSet::with_capacity(capacity),
            capacity,
        }
    }

    /// Check if this message was already seen. If new, insert it.
    ///
    /// Returns `true` if **duplicate** (already in cache).
    /// Returns `false` if **new** (inserted into cache).
    pub fn check_and_insert(&mut self, community_id: &str, sender: &str, dedup_key: &str) -> bool {
        let hash = hash_dedup_tuple(community_id, sender, dedup_key);
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

    /// Current number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Hash a dedup tuple to its 64-bit cache key.
///
/// Each field is length-prefixed rather than delimited, so no choice of
/// field contents can make `("ab", "c")` hash as `("a", "bc")`. A
/// separator byte would be defeated by a field that legitimately
/// contains it; a length prefix cannot alias at all.
fn hash_dedup_tuple(community_id: &str, sender: &str, dedup_key: &str) -> DedupHash {
    let mut hasher = blake3::Hasher::new();
    for field in [community_id, sender, dedup_key] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    let digest = hasher.finalize();
    u64::from_le_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("Blake3 always produces >=8 bytes"),
    )
}

/// The dedup key of a gossip envelope: its message id, hex-encoded.
///
/// The id is chosen by the sender per send, covered by the signature and
/// forwarded unchanged by relays, so every copy of one message shares it
/// while a re-send of identical bytes does not. Content-derived keys (a
/// hash of the payload, a time bucket) were used before and silently
/// dropped deliberate re-sends; the libp2p pubsub spec names the same
/// hazard for content-addressed ids. The sender pseudonym is supplied to
/// [`DedupCache::check_and_insert`] separately, so an id can never
/// collide across senders.
#[must_use]
pub fn message_dedup_key(signed: &SignedEnvelope) -> String {
    hex::encode(signed.message_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_cache_rejects_duplicates() {
        let mut cache = DedupCache::new(10);
        assert!(!cache.check_and_insert("c1", "sender1", "msg1"));
        assert!(cache.check_and_insert("c1", "sender1", "msg1"));
    }

    #[test]
    fn dedup_cache_different_keys_accepted() {
        let mut cache = DedupCache::new(10);
        assert!(!cache.check_and_insert("c1", "s1", "msg1"));
        assert!(!cache.check_and_insert("c1", "s1", "msg2"));
        assert!(!cache.check_and_insert("c1", "s2", "msg1"));
        assert!(!cache.check_and_insert("c2", "s1", "msg1"));
    }

    #[test]
    fn dedup_cache_evicts_oldest_at_capacity() {
        let mut cache = DedupCache::new(3);
        cache.check_and_insert("c", "s", "a");
        cache.check_and_insert("c", "s", "b");
        cache.check_and_insert("c", "s", "c");
        assert_eq!(cache.len(), 3);

        // Insert 4th — should evict "a"
        cache.check_and_insert("c", "s", "d");
        assert_eq!(cache.len(), 3);
        // "a" should no longer be recognized as duplicate
        assert!(!cache.check_and_insert("c", "s", "a"));
    }

    #[test]
    fn dedup_cache_clear() {
        let mut cache = DedupCache::new(10);
        cache.check_and_insert("c", "s", "msg");
        cache.clear();
        assert!(cache.is_empty());
        // Previously seen message is now new
        assert!(!cache.check_and_insert("c", "s", "msg"));
    }

    fn signed(message_id: [u8; 16], payload: &[u8]) -> SignedEnvelope {
        SignedEnvelope {
            community_id: "c".into(),
            sender_pseudonym: "aabb".into(),
            message_id,
            envelope_bytes: payload.to_vec(),
            signature: vec![0; 64],
            ttl: 5,
        }
    }

    /// One message relayed twice is one key; the same bytes re-sent
    /// under a new id are a new key (the roster reconcile's re-send).
    #[test]
    fn dedup_key_follows_the_message_id_not_the_content() {
        let first = signed([1; 16], b"join");
        let mut relayed = first.clone();
        relayed.ttl -= 1;
        let resent = signed([2; 16], b"join");
        assert_eq!(message_dedup_key(&first), message_dedup_key(&relayed));
        assert_ne!(message_dedup_key(&first), message_dedup_key(&resent));
        assert_eq!(message_dedup_key(&first).len(), 32);
    }
}
