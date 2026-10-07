//! Phase 17 — concrete `InMemoryMekCache` implementing `ChannelMekCache`.
//!
//! Wraps `parking_lot::Mutex<HashMap<(String, KeyScope), MediaEncryptionKey>>`
//! keyed by `(community_id, scope)`, holding each scope's current key.
//! Generation-matched reads: `get(community, scope, generation)` only
//! returns the cached MEK when its generation matches.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;

use crate::deps::ChannelMekCache;

/// Each `(community, scope)`'s current key.
type Entries = HashMap<(String, KeyScope), MediaEncryptionKey>;

/// Thread-safe in-memory MEK cache. Cheaply clonable (Arc-backed).
#[derive(Clone, Default)]
pub struct InMemoryMekCache {
    inner: Arc<Mutex<Entries>>,
}

impl InMemoryMekCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the (community, scope) → generation mapping for
    /// diagnostics. Each pair appears once with its current generation.
    #[must_use]
    pub fn snapshot_generations(&self) -> Vec<((String, KeyScope), u64)> {
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
    fn current(&self, community_id: &str, scope: KeyScope) -> Option<MediaEncryptionKey> {
        self.inner
            .lock()
            .get(&(community_id.to_string(), scope))
            .cloned()
    }

    fn insert(&self, community_id: &str, scope: KeyScope, mek: MediaEncryptionKey) -> bool {
        let mut map = self.inner.lock();
        let key = (community_id.to_string(), scope);
        if let Some(cached) = map.get(&key) {
            if mek.generation() < cached.generation() {
                return false;
            }
            if cached.generation() == mek.generation()
                && !crate::convergence::incoming_wins_same_generation(
                    cached.election_rank().as_ref(),
                    mek.election_rank().as_ref(),
                )
            {
                return false;
            }
        }
        map.insert(key, mek);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::ChannelId;

    const CH1: KeyScope = KeyScope::Channel(ChannelId([1; 16]));
    const CH2: KeyScope = KeyScope::Channel(ChannelId([2; 16]));
    const CH3: KeyScope = KeyScope::Channel(ChannelId([3; 16]));

    fn mek(generation: u64) -> MediaEncryptionKey {
        let mut bytes = [0u8; 32];
        bytes[0] = u8::try_from(generation & 0xff).unwrap();
        MediaEncryptionKey::from_bytes(bytes, generation)
    }

    #[test]
    fn empty_cache_returns_none_on_get_and_zero_generation() {
        let cache = InMemoryMekCache::new();
        assert!(cache.get("c1", CH1, 1).is_none());
        assert_eq!(cache.current_generation("c1", CH1), 0);
    }

    #[test]
    fn insert_then_get_at_matching_generation_returns_mek() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(5));
        match cache.get("c1", CH1, 5) {
            Some(m) => assert_eq!(m.generation(), 5),
            None => panic!("expected MEK at generation 5"),
        }
    }

    #[test]
    fn get_at_mismatched_generation_returns_none() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(5));
        assert!(cache.get("c1", CH1, 4).is_none());
        assert!(cache.get("c1", CH1, 6).is_none());
    }

    #[test]
    fn current_generation_returns_cached_value() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(7));
        assert_eq!(cache.current_generation("c1", CH1), 7);
    }

    #[test]
    fn insert_overwrites_previous_generation() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(1));
        cache.insert("c1", CH1, mek(2));
        assert_eq!(cache.current_generation("c1", CH1), 2);
        assert!(cache.get("c1", CH1, 1).is_none());
        assert!(cache.get("c1", CH1, 2).is_some());
    }

    #[test]
    fn older_generation_is_refused() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(5));
        cache.insert("c1", CH1, mek(4));
        assert_eq!(cache.current_generation("c1", CH1), 5);
    }

    #[test]
    fn community_scope_is_its_own_entry() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", KeyScope::Community, mek(3));
        cache.insert("c1", CH1, mek(1));
        assert_eq!(cache.current_generation("c1", KeyScope::Community), 3);
        assert_eq!(cache.current_generation("c1", CH1), 1);
        assert!(cache.get("c1", CH1, 3).is_none());
    }

    #[test]
    fn different_community_channel_pairs_isolated() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(5));
        cache.insert("c2", CH1, mek(10));
        cache.insert("c1", CH2, mek(15));
        assert_eq!(cache.current_generation("c1", CH1), 5);
        assert_eq!(cache.current_generation("c2", CH1), 10);
        assert_eq!(cache.current_generation("c1", CH2), 15);
    }

    #[test]
    fn clear_removes_all_entries() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(1));
        cache.insert("c2", CH2, mek(2));
        cache.clear();
        assert_eq!(cache.current_generation("c1", CH1), 0);
        assert_eq!(cache.current_generation("c2", CH2), 0);
    }

    #[test]
    fn snapshot_returns_all_pairs() {
        let cache = InMemoryMekCache::new();
        cache.insert("c1", CH1, mek(5));
        cache.insert("c2", CH3, mek(8));
        let snap = cache.snapshot_generations();
        assert_eq!(snap.len(), 2);
        assert!(snap
            .iter()
            .any(|(k, g)| k == &("c1".into(), CH1) && *g == 5));
        assert!(snap
            .iter()
            .any(|(k, g)| k == &("c2".into(), CH3) && *g == 8));
    }

    #[test]
    fn same_generation_converges_on_lowest_election_rank() {
        // Two rotators minted different keys at the SAME generation. Whichever
        // order they arrive, the cache must end up holding the lower-rank key.
        let low =
            MediaEncryptionKey::from_bytes([1u8; 32], 5).with_provenance([0u8; 32], [0x10; 32]);
        let high =
            MediaEncryptionKey::from_bytes([2u8; 32], 5).with_provenance([0u8; 32], [0x20; 32]);

        let cache = InMemoryMekCache::new();
        cache.insert("c", CH1, high.clone());
        cache.insert("c", CH1, low.clone());
        assert_eq!(cache.get("c", CH1, 5).unwrap().as_bytes(), low.as_bytes());

        let cache2 = InMemoryMekCache::new();
        cache2.insert("c", CH1, low.clone());
        cache2.insert("c", CH1, high.clone());
        assert_eq!(cache2.get("c", CH1, 5).unwrap().as_bytes(), low.as_bytes());
    }

    #[test]
    fn clone_shares_inner_state() {
        let cache = InMemoryMekCache::new();
        let other = cache.clone();
        cache.insert("c1", CH1, mek(11));
        assert_eq!(other.current_generation("c1", CH1), 11);
    }
}
