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
#[path = "dedup_tests.rs"]
mod tests;
