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

/// Thread-safe in-memory MEK cache. Cheaply cloneable (Arc-backed).
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
#[path = "cache_tests.rs"]
mod tests;
