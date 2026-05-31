//! Phase 17 — concrete `InMemoryMekCache` implementing `ChannelMekCache`.
//!
//! Wraps `parking_lot::Mutex<HashMap<(String, String), MediaEncryptionKey>>`
//! keyed by `(community_id, channel_id)`. Generation-matched reads:
//! `get(community, channel, generation)` only returns the cached MEK
//! when its generation matches; mismatches return None so callers know
//! to fall through to durable storage.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_crypto::group::media_key::MediaEncryptionKey;

use crate::deps::ChannelMekCache;

/// Thread-safe in-memory MEK cache. Cheaply clonable (Arc-backed).
#[derive(Clone, Default)]
pub struct InMemoryMekCache {
    inner: Arc<Mutex<HashMap<(String, String), MediaEncryptionKey>>>,
}

impl InMemoryMekCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the (community, channel) → generation mapping for
    /// diagnostics / cold-start replay. Each pair appears once with
    /// the currently-cached generation.
    #[must_use]
    pub fn snapshot_generations(&self) -> Vec<((String, String), u64)> {
        self.inner
            .lock()
            .iter()
            .map(|(key, mek)| (key.clone(), mek.generation()))
            .collect()
    }

    /// Drop all cached MEKs. Used on logout.
    pub fn clear(&self) {
        self.inner.lock().clear();
    }
}

impl ChannelMekCache for InMemoryMekCache {
    fn get(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let map = self.inner.lock();
        let mek = map.get(&(community_id.to_string(), channel_id.to_string()))?;
        if mek.generation() == generation {
            Some(mek.clone())
        } else {
            None
        }
    }

    fn insert(&self, community_id: &str, channel_id: &str, mek: MediaEncryptionKey) {
        self.inner
            .lock()
            .insert((community_id.to_string(), channel_id.to_string()), mek);
    }

    fn current_generation(&self, community_id: &str, channel_id: &str) -> u64 {
        self.inner
            .lock()
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map_or(0, MediaEncryptionKey::generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mek(generation: u64) -> MediaEncryptionKey {
        let mut bytes = [0u8; 32];
        bytes[0] = u8::try_from(generation & 0xff).unwrap();
        MediaEncryptionKey::from_bytes(bytes, generation)
    }

    #[test]
    fn empty_cache_returns_none_on_get_and_zero_generation() {
        let cache = InMemoryMekCache::new();
        assert!(cache.get("c1", "ch1", 1).is_none());
        assert_eq!(cache.current_generation("c1", "ch1"), 0);
    }

    #[test]
    fn insert_then_get_at_matching_generation_returns_mek() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(5));
        match cache.get("c1", "ch1", 5) {
            Some(m) => assert_eq!(m.generation(), 5),
            None => panic!("expected MEK at generation 5"),
        }
    }

    #[test]
    fn get_at_mismatched_generation_returns_none() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(5));
        assert!(cache.get("c1", "ch1", 4).is_none());
        assert!(cache.get("c1", "ch1", 6).is_none());
    }

    #[test]
    fn current_generation_returns_cached_value() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(7));
        assert_eq!(cache.current_generation("c1", "ch1"), 7);
    }

    #[test]
    fn insert_overwrites_previous_generation() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(1));
        cache.insert("c1", "ch1", mek(2));
        assert_eq!(cache.current_generation("c1", "ch1"), 2);
        assert!(cache.get("c1", "ch1", 1).is_none());
        assert!(cache.get("c1", "ch1", 2).is_some());
    }

    #[test]
    fn different_community_channel_pairs_isolated() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(5));
        cache.insert("c2", "ch1", mek(10));
        cache.insert("c1", "ch2", mek(15));
        assert_eq!(cache.current_generation("c1", "ch1"), 5);
        assert_eq!(cache.current_generation("c2", "ch1"), 10);
        assert_eq!(cache.current_generation("c1", "ch2"), 15);
    }

    #[test]
    fn clear_removes_all_entries() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(1));
        cache.insert("c2", "ch2", mek(2));
        cache.clear();
        assert_eq!(cache.current_generation("c1", "ch1"), 0);
        assert_eq!(cache.current_generation("c2", "ch2"), 0);
    }

    #[test]
    fn snapshot_returns_all_pairs() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", "ch1", mek(5));
        cache.insert("c2", "ch3", mek(8));
        let snap = cache.snapshot_generations();
        assert_eq!(snap.len(), 2);
        assert!(snap
            .iter()
            .any(|(k, g)| k == &("c1".into(), "ch1".into()) && *g == 5));
        assert!(snap
            .iter()
            .any(|(k, g)| k == &("c2".into(), "ch3".into()) && *g == 8));
    }

    #[test]
    fn clone_shares_inner_state() {
        let cache = InMemoryMekCache::new();
        let other = cache.clone();
        cache.insert("c1", "ch1", mek(11));
        assert_eq!(other.current_generation("c1", "ch1"), 11);
    }
}
