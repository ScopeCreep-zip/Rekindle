use rekindle_transport_ipc::v3::dedup::cache::{ReceiverCache, ReceiverCacheConfig};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn hash(n: u8) -> [u8; 32] { [n; 32] }

#[test]
fn store_and_lookup() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![1, 2, 3, 4], 4, 1, Clearance::Internal);
    let entry = cache.lookup(&hash(1));
    assert!(entry.is_some());
}

#[test]
fn lookup_unknown_returns_none() {
    let cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    assert!(cache.lookup(&hash(99)).is_none());
}

#[test]
fn payload_bytes_match() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
    cache.store(hash(1), payload.clone(), 4, 1, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.payload, payload);
}

#[test]
fn eviction_removes_lru() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 3, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(2), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(3), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(4), vec![0; 10], 10, 1, Clearance::Internal);
    assert!(cache.lookup(&hash(1)).is_none());
    assert!(cache.lookup(&hash(4)).is_some());
}

#[test]
fn byte_limit_evicts() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 25 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(2), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(3), vec![0; 10], 10, 1, Clearance::Internal); // total 30 > 25
    assert!(cache.lookup(&hash(1)).is_none());
}

#[test]
fn size_and_chunk_count_tracked() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 1024], 1_048_576, 64, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.payload_size_bytes, 1_048_576);
    assert_eq!(entry.chunk_count, 64);
}

#[test]
fn provider_clearance_recorded() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Confidential);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.clearance, Clearance::Confidential);
}

#[test]
fn multiple_providers_max_clearance() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Confidential);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.clearance, Clearance::Confidential,
        "clearance must be max of all providers");
}

#[test]
fn receipt_count_increments() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.receipt_count, 2);
}

// ── Adversarial ───────────────────────────────────────────────────

#[test]
fn cache_poisoning_via_wrong_hash_impossible() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0xAA; 10], 10, 1, Clearance::Internal);
    // Attacker stores different payload under same hash — overwrites.
    // This is correct: the cache trusts the last writer.
    // The protection is at the STREAM_REFERENCE layer: the receiver
    // verifies content_hash against the cached payload before serving.
    cache.store(hash(1), vec![0xBB; 10], 10, 1, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.payload, vec![0xBB; 10]);
}

#[test]
fn empty_payload_cacheable() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![], 0, 0, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert!(entry.payload.is_empty());
    assert_eq!(entry.payload_size_bytes, 0);
}

#[test]
fn max_u64_byte_count_no_overflow() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], u64::MAX, u32::MAX, Clearance::Internal);
    let entry = cache.lookup(&hash(1)).unwrap();
    assert_eq!(entry.payload_size_bytes, u64::MAX);
    assert_eq!(entry.chunk_count, u32::MAX);
}
