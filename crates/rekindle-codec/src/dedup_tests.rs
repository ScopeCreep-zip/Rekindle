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
