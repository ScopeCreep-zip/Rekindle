use std::time::{Duration, Instant};

use super::RouteCache;

#[test]
fn evicts_only_stale_routes() {
    let start = Instant::now();
    let mut cache = RouteCache::new(Duration::from_secs(120));
    cache.insert_at("alice", vec![1], start);
    cache.insert_at("bob", vec![2], start + Duration::from_secs(90));

    let evicted = cache.evict_stale_at(start + Duration::from_secs(121));
    assert_eq!(evicted, vec!["alice".to_string()]);
    assert!(cache.get("alice").is_none());
    assert!(cache.get("bob").is_some());
}

#[test]
fn remove_by_blob_drops_only_peers_on_dead_routes() {
    let now = Instant::now();
    let mut cache = RouteCache::new(Duration::from_secs(60));
    cache.insert_at("alice", vec![1], now);
    cache.insert_at("bob", vec![2], now);
    let dead = cache.remove_by_blob(&[vec![1], vec![9]]);
    assert_eq!(dead, vec!["alice".to_string()]);
    assert!(cache.get("alice").is_none());
    assert!(cache.get("bob").is_some());
}
