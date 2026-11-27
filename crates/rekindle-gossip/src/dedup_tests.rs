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
