use rekindle_transport_ipc::v3::dedup::cache::{
    SenderCache, SenderCacheConfig, ReceiverCache, ReceiverCacheConfig,
};
use rekindle_transport_ipc::v3::dedup::reference::{
    sender_decision, receiver_decision,
    SenderDecision, ReceiverDecision,
};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

fn hash(n: u8) -> [u8; 32] { [n; 32] }
fn sid(n: u128) -> uuid::Uuid { uuid::Uuid::from_u128(n) }
fn pid(n: u8) -> [u8; 32] { [n; 32] }

#[test]
fn novel_content_returns_send_full() {
    let cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    let decision = sender_decision(&cache, &hash(1), sid(1), pid(1));
    assert!(matches!(decision, SenderDecision::SendFull));
}

#[test]
fn cached_and_acked_returns_send_reference() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    cache.mark_acked(&hash(1), sid(1), pid(1));
    let decision = sender_decision(&cache, &hash(1), sid(1), pid(1));
    assert!(matches!(decision, SenderDecision::SendReference));
}

#[test]
fn cached_but_not_acked_returns_send_full() {
    let mut cache = SenderCache::new(SenderCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), 1024, 1);
    let decision = sender_decision(&cache, &hash(1), sid(1), pid(1));
    assert!(matches!(decision, SenderDecision::SendFull));
}

#[test]
fn receiver_cache_hit_serves_from_cache() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0xAA; 100], 100, 1, Clearance::Internal);
    let decision = receiver_decision(&cache, &hash(1), 100, 1, Clearance::Internal);
    match decision {
        ReceiverDecision::CacheHit { payload } => {
            assert_eq!(payload, vec![0xAA; 100]);
        }
        other => panic!("Expected CacheHit, got {other:?}"),
    }
}

#[test]
fn receiver_cache_miss_returns_miss() {
    let cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    let decision = receiver_decision(&cache, &hash(1), 100, 1, Clearance::Internal);
    assert!(matches!(decision, ReceiverDecision::CacheMiss));
}

#[test]
fn receiver_cache_size_mismatch_returns_mismatch() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 100], 100, 1, Clearance::Internal);
    let decision = receiver_decision(&cache, &hash(1), 999, 1, Clearance::Internal); // wrong size
    assert!(matches!(decision, ReceiverDecision::CacheMismatch));
}

#[test]
fn receiver_cache_chunk_mismatch_returns_mismatch() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 100, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 100], 100, 1, Clearance::Internal);
    let decision = receiver_decision(&cache, &hash(1), 100, 99, Clearance::Internal); // wrong chunks
    assert!(matches!(decision, ReceiverDecision::CacheMismatch));
}

// ── Adversarial ───────────────────────────────────────────────────

#[test]
fn reference_for_evicted_entry_returns_miss() {
    let mut cache = ReceiverCache::new(ReceiverCacheConfig { max_entries: 2, max_bytes: 1_000_000 });
    cache.store(hash(1), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(2), vec![0; 10], 10, 1, Clearance::Internal);
    cache.store(hash(3), vec![0; 10], 10, 1, Clearance::Internal); // evicts hash(1)
    let decision = receiver_decision(&cache, &hash(1), 10, 1, Clearance::Internal);
    assert!(matches!(decision, ReceiverDecision::CacheMiss));
}
