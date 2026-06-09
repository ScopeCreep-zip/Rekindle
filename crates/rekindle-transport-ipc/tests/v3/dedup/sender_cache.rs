use rekindle_transport_ipc::v3::dedup::cache::{SenderCache, SenderCacheConfig};

fn hash(n: u8) -> [u8; 32] { [n; 32] }
fn sid(n: u128) -> uuid::Uuid { uuid::Uuid::from_u128(n) }
fn pid(n: u8) -> [u8; 32] { [n; 32] }

#[test]
fn store_and_lookup() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    let entry = cache.lookup(&hash(1));
    assert!(entry.is_some());
    let e = entry.unwrap();
    assert_eq!(e.payload_size_bytes, 1024);
    assert_eq!(e.chunk_count, 1);
}

#[test]
fn lookup_unknown_returns_none() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    assert!(cache.lookup(&hash(99)).is_none());
}

#[test]
fn mark_receiver_acked() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    cache.mark_acked(&hash(1), sid(1), pid(1));
    assert!(cache.is_acked(&hash(1), sid(1), pid(1)));
}

#[test]
fn unacked_receiver_not_reported() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    assert!(!cache.is_acked(&hash(1), sid(1), pid(1)));
}

#[test]
fn multiple_receivers_tracked() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    cache.mark_acked(&hash(1), sid(1), pid(1));
    cache.mark_acked(&hash(1), sid(2), pid(2));
    cache.mark_acked(&hash(1), sid(3), pid(3));
    assert!(cache.is_acked(&hash(1), sid(1), pid(1)));
    assert!(cache.is_acked(&hash(1), sid(2), pid(2)));
    assert!(cache.is_acked(&hash(1), sid(3), pid(3)));
    assert!(!cache.is_acked(&hash(1), sid(4), pid(4)));
}

#[test]
fn eviction_removes_lru() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 3, max_bytes: 1_000_000 });
    cache.store(hash(1), 100, 1);
    cache.store(hash(2), 100, 1);
    cache.store(hash(3), 100, 1);
    cache.store(hash(4), 100, 1); // evicts hash(1)
    assert!(cache.lookup(&hash(1)).is_none());
    assert!(cache.lookup(&hash(2)).is_some());
    assert!(cache.lookup(&hash(3)).is_some());
    assert!(cache.lookup(&hash(4)).is_some());
}

#[test]
fn access_refreshes_lru_position() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 3, max_bytes: 1_000_000 });
    cache.store(hash(1), 100, 1);
    cache.store(hash(2), 100, 1);
    cache.store(hash(3), 100, 1);
    // Access hash(1) to refresh it
    cache.lookup(&hash(1));
    // Insert hash(4) — should evict hash(2) (oldest untouched), not hash(1)
    cache.store(hash(4), 100, 1);
    assert!(cache.lookup(&hash(1)).is_some(), "refreshed entry must survive");
    assert!(cache.lookup(&hash(2)).is_none(), "LRU entry must be evicted");
}

#[test]
fn byte_limit_evicts() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 250 });
    cache.store(hash(1), 100, 1);
    cache.store(hash(2), 100, 1);
    cache.store(hash(3), 100, 1); // total 300 > 250, evicts hash(1)
    assert!(cache.lookup(&hash(1)).is_none());
    assert!(cache.lookup(&hash(3)).is_some());
}

#[test]
fn emission_count_tracked() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 100, 1);
    cache.record_emission(&hash(1));
    cache.record_emission(&hash(1));
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.emission_count, 2);
}

#[test]
fn session_close_removes_acked_peer() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 100, 1);
    cache.mark_acked(&hash(1), sid(1), pid(1));
    assert!(cache.is_acked(&hash(1), sid(1), pid(1)));
    cache.close_session(sid(1));
    assert!(!cache.is_acked(&hash(1), sid(1), pid(1)));
}

// ── Adversarial ───────────────────────────────────────────────────

#[test]
fn zero_capacity_cache_never_caches() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 0, max_bytes: 0 });
    cache.store(hash(1), 100, 1);
    assert!(cache.lookup(&hash(1)).is_none());
}

#[test]
fn concurrent_store_and_evict_consistent() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 2, max_bytes: 1_000_000 });
    for i in 0..100u8 {
        cache.store(hash(i), 100, 1);
    }
    // After 100 insertions with capacity 2, exactly 2 remain
    let mut count = 0;
    for i in 0..100u8 {
        if cache.lookup(&hash(i)).is_some() {
            count += 1;
        }
    }
    assert_eq!(count, 2);
}
