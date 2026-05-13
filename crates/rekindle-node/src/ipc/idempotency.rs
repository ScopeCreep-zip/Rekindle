//! Request idempotency cache for exactly-once semantics.
//!
//! Bounded LRU with TTL-based eviction. O(1) insert, O(1) lookup,
//! O(1) eviction. Cache size: 10,000 entries. TTL: 5 minutes.

use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;

use super::protocol::IpcResponse;

const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_CAPACITY: usize = 10_000;

struct CacheEntry {
    response: IpcResponse,
    inserted_at: Instant,
}

/// LRU idempotency cache. O(1) eviction.
pub struct IdempotencyCache {
    cache: Mutex<LruCache<String, CacheEntry>>,
}

impl IdempotencyCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).expect("non-zero"),
            )),
        }
    }

    /// Check if a request with this ID was already processed.
    /// Returns the cached response if found and not expired.
    pub fn check(&self, client_msg_id: &str) -> Option<IpcResponse> {
        let mut cache = self.cache.lock();
        let entry = cache.get(client_msg_id)?;
        if entry.inserted_at.elapsed() < CACHE_TTL {
            Some(entry.response.clone())
        } else {
            cache.pop(client_msg_id);
            None
        }
    }

    /// Store a processed request's response in the cache.
    pub fn store(&self, client_msg_id: String, response: IpcResponse) {
        self.cache.lock().put(client_msg_id, CacheEntry {
            response,
            inserted_at: Instant::now(),
        });
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.lock().is_empty()
    }
}

impl Default for IdempotencyCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miss_then_hit() {
        let cache = IdempotencyCache::new();
        assert!(cache.check("req-1").is_none());
        cache.store("req-1".into(), IpcResponse::ok(&serde_json::json!({"sent": true})));
        assert!(cache.check("req-1").is_some());
    }

    #[test]
    fn different_ids_independent() {
        let cache = IdempotencyCache::new();
        cache.store("a".into(), IpcResponse::ok(&serde_json::json!(1)));
        cache.store("b".into(), IpcResponse::ok(&serde_json::json!(2)));
        assert!(cache.check("a").is_some());
        assert!(cache.check("b").is_some());
        assert!(cache.check("c").is_none());
    }

    #[test]
    fn capacity_eviction() {
        let cache = IdempotencyCache::new();
        for i in 0..CACHE_CAPACITY + 100 {
            cache.store(format!("req-{i}"), IpcResponse::ok(&serde_json::json!(i)));
        }
        assert!(cache.len() <= CACHE_CAPACITY);
    }
}
