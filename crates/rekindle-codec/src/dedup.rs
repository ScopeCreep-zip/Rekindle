//! Gossip message deduplication cache and key extraction.
//!
//! Prevents infinite gossip forwarding loops and duplicate processing.
//! FIFO eviction when capacity is exceeded. Different envelope types
//! use different dedup strategies (exact ID vs time-bucketed).

use blake2::{digest::consts::U16, Blake2b, Digest};
use indexmap::IndexMap;

use crate::envelope::SignedEnvelope;

/// FIFO dedup cache — returns true if a message was already seen.
///
/// Key: `(community_id, sender_pseudonym, dedup_key)`.
/// Capacity: 1024 entries (covers ~100s of traffic at 10 msgs/sec).
pub struct DedupCache {
    entries: IndexMap<(String, String, String), ()>,
    capacity: usize,
}

impl DedupCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: IndexMap::with_capacity(capacity),
            capacity,
        }
    }

    /// Check if this message was already seen. If new, insert it.
    ///
    /// Returns `true` if **duplicate** (already in cache).
    /// Returns `false` if **new** (inserted into cache).
    pub fn check_and_insert(
        &mut self,
        community_id: &str,
        sender: &str,
        dedup_key: &str,
    ) -> bool {
        let key = (
            community_id.to_string(),
            sender.to_string(),
            dedup_key.to_string(),
        );
        if self.entries.contains_key(&key) {
            return true;
        }
        if self.entries.len() >= self.capacity {
            self.entries.shift_remove_index(0);
        }
        self.entries.insert(key, ());
        false
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
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

/// Extract a dedup key from a signed envelope's inner payload.
///
/// Different envelope types use different strategies:
/// - Known message types: extract a unique ID field.
/// - Unknown/fallback: BLAKE2b-128 hash of envelope bytes (collision-resistant).
///
/// This function deserializes the envelope to inspect it. If deserialization
/// fails, it falls back to hashing.
///
/// # Note
/// This operates on the `SignedEnvelope`'s `envelope_bytes` which contains
/// the JSON-serialized inner payload. The sender pseudonym is NOT part of
/// the dedup key — it's provided separately to `check_and_insert`.
pub fn extract_dedup_key(signed: &SignedEnvelope) -> String {
    // Try to extract a meaningful dedup key from known fields.
    // We peek at the JSON to find a "type" tag and relevant ID fields
    // without requiring the full envelope type to be in this crate.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&signed.envelope_bytes) {
        // Check for message_id (ChatMessage, governance entries, etc.)
        if let Some(id) = value.get("message_id").and_then(|v| v.as_str()) {
            return id.to_string();
        }

        // Check for governance entry type + lamport (unique per author per lamport)
        if let Some(entry_type) = value.get("type").and_then(|v| v.as_str()) {
            if let Some(lamport) = value.get("lamport").and_then(|v| v.as_u64()) {
                return format!("{entry_type}:{lamport}");
            }
        }
    }

    // Fallback: BLAKE2b-128 hash of envelope bytes
    hash_envelope_bytes(&signed.envelope_bytes)
}

/// BLAKE2b-128 hash of arbitrary bytes, hex-encoded.
///
/// Used as a collision-resistant dedup key when no semantic ID is available.
pub fn hash_envelope_bytes(bytes: &[u8]) -> String {
    let mut h = Blake2b::<U16>::new();
    h.update(bytes);
    hex::encode(h.finalize())
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

    #[test]
    fn extract_dedup_key_from_governance_entry() {
        let payload = serde_json::json!({
            "type": "channel_created",
            "channel_id": [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16],
            "name": "general",
            "channel_type": "text",
            "record_key": "VLD0:abc",
            "category_id": null,
            "position": 0,
            "lamport": 42
        });
        let signed = SignedEnvelope {
            community_id: "c".into(),
            sender_pseudonym: "aabb".into(),
            envelope_bytes: serde_json::to_vec(&payload).unwrap(),
            signature: vec![0; 64],
            ttl: 5,
        };
        let key = extract_dedup_key(&signed);
        assert_eq!(key, "channel_created:42");
    }

    #[test]
    fn extract_dedup_key_fallback_to_hash() {
        let signed = SignedEnvelope {
            community_id: "c".into(),
            sender_pseudonym: "aabb".into(),
            envelope_bytes: vec![0xFF, 0xFE], // not valid JSON
            signature: vec![0; 64],
            ttl: 5,
        };
        let key = extract_dedup_key(&signed);
        // Should be a hex-encoded BLAKE2b hash
        assert_eq!(key.len(), 32); // 16 bytes = 32 hex chars
    }

    #[test]
    fn hash_deterministic() {
        let h1 = hash_envelope_bytes(b"hello");
        let h2 = hash_envelope_bytes(b"hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_differs() {
        let h1 = hash_envelope_bytes(b"hello");
        let h2 = hash_envelope_bytes(b"world");
        assert_ne!(h1, h2);
    }
}
