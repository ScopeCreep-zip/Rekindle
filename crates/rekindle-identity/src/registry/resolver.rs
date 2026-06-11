//! PeerResolver — remote peer resolution cache.
//!
//! Bounded LRU cache mapping `PeerRef` → (LocatorRecord, cached persona,
//! trust snapshot). DashMap-sharded for concurrent access.
//!
//! Design point: 1M entries maximum. Entries are populated on first
//! contact with a remote peer and refreshed on locator epoch changes.

use std::collections::VecDeque;
use std::sync::Mutex;

use dashmap::DashMap;

use crate::locator::record::LocatorRecord;
use crate::root::PeerRef;
use crate::trust::TrustState;

/// Default cache capacity (1M entries).
const DEFAULT_CAPACITY: usize = 1_000_000;

/// A cached entry for a remote peer.
#[derive(Debug, Clone)]
pub struct ResolvedPeer {
    /// The peer's latest known locator record.
    pub locators: Option<LocatorRecord>,
    /// The peer's display name (from persona or gossip).
    pub display_name: Option<String>,
    /// The peer's trust state (cached snapshot, not authoritative —
    /// authoritative state is in TrustStore).
    pub trust_state: Option<TrustState>,
}

/// Remote peer resolution cache. Bounded LRU, DashMap-sharded.
///
/// The `insert` method uses the DashMap `entry` API to make
/// presence-check and insertion atomic, preventing the TOCTOU race
/// where two threads both see the key as absent and both insert.
pub struct PeerResolver {
    entries: DashMap<PeerRef, ResolvedPeer>,
    /// LRU eviction order. The mutex serializes insert/eviction.
    /// Reads (`get`) never touch this.
    lru_order: Mutex<VecDeque<PeerRef>>,
    capacity: usize,
}

impl PeerResolver {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: DashMap::new(),
            lru_order: Mutex::new(VecDeque::with_capacity(capacity.min(65536))),
            capacity,
        }
    }

    /// Look up a cached peer (lock-free read).
    pub fn get(&self, peer: &PeerRef) -> Option<ResolvedPeer> {
        self.entries.get(peer).map(|entry| entry.clone())
    }

    /// Insert or update a cached peer. Evicts LRU if at capacity.
    ///
    /// Uses the DashMap entry API to make the new-vs-update decision
    /// atomic — no TOCTOU between contains_key and insert.
    pub fn insert(&self, peer: PeerRef, resolved: ResolvedPeer) {
        let mut lru = self.lru_order.lock().expect("lru lock poisoned");

        // Phase 1: evict if at capacity BEFORE acquiring the entry guard.
        // This prevents the same-shard self-deadlock where entry(peer) holds
        // a shard lock and remove(&evicted) tries to acquire the same shard.
        if !self.entries.contains_key(&peer) && lru.len() >= self.capacity {
            if let Some(evicted) = lru.pop_front() {
                self.entries.remove(&evicted);
            }
        }

        // Phase 2: insert or update. No eviction under this guard.
        match self.entries.entry(peer) {
            dashmap::mapref::entry::Entry::Vacant(v) => {
                lru.push_back(peer);
                v.insert(resolved);
            }
            dashmap::mapref::entry::Entry::Occupied(mut o) => {
                o.insert(resolved);
            }
        }
    }

    /// Update the locator record for a peer.
    pub fn update_locators(&self, peer: &PeerRef, locators: LocatorRecord) {
        if let Some(mut entry) = self.entries.get_mut(peer) {
            entry.locators = Some(locators);
        }
    }

    /// Update the display name for a peer.
    pub fn update_display_name(&self, peer: &PeerRef, name: String) {
        if let Some(mut entry) = self.entries.get_mut(peer) {
            entry.display_name = Some(name);
        }
    }

    /// Update the cached trust state for a peer.
    pub fn update_trust(&self, peer: &PeerRef, state: TrustState) {
        if let Some(mut entry) = self.entries.get_mut(peer) {
            entry.trust_state = Some(state);
        }
    }

    /// Remove a peer from the cache (on identity death or ban).
    pub fn remove(&self, peer: &PeerRef) -> Option<ResolvedPeer> {
        let removed = self.entries.remove(peer);
        if removed.is_some() {
            let mut lru = self.lru_order.lock().expect("lru lock poisoned");
            lru.retain(|p| p != peer);
        }
        removed.map(|(_, v)| v)
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn capacity(&self) -> usize { self.capacity }
}

impl Default for PeerResolver {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_peer(byte: u8) -> PeerRef {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
        ).unwrap();
        PeerRef::anchor(o.root)
    }

    fn empty_resolved() -> ResolvedPeer {
        ResolvedPeer {
            locators: None,
            display_name: None,
            trust_state: None,
        }
    }

    #[test]
    fn insert_and_get() {
        let resolver = PeerResolver::new();
        let peer = make_peer(0x01);
        resolver.insert(peer, ResolvedPeer {
            locators: None,
            display_name: Some("alice".into()),
            trust_state: Some(TrustState::Pinned),
        });

        let found = resolver.get(&peer).unwrap();
        assert_eq!(found.display_name.as_deref(), Some("alice"));
    }

    #[test]
    fn lru_eviction() {
        let resolver = PeerResolver::with_capacity(4);

        let peers: Vec<_> = (0..5u8).map(make_peer).collect();
        for &peer in &peers {
            resolver.insert(peer, empty_resolved());
        }

        assert_eq!(resolver.len(), 4);
        assert!(resolver.get(&peers[0]).is_none(), "oldest entry must be evicted");
        assert!(resolver.get(&peers[4]).is_some(), "newest entry must survive");
    }

    #[test]
    fn update_display_name() {
        let resolver = PeerResolver::new();
        let peer = make_peer(0x01);
        resolver.insert(peer, empty_resolved());

        resolver.update_display_name(&peer, "bob".into());
        assert_eq!(resolver.get(&peer).unwrap().display_name.as_deref(), Some("bob"));
    }

    #[test]
    fn update_trust() {
        let resolver = PeerResolver::new();
        let peer = make_peer(0x01);
        resolver.insert(peer, empty_resolved());

        resolver.update_trust(&peer, TrustState::Verified);
        assert_eq!(resolver.get(&peer).unwrap().trust_state, Some(TrustState::Verified));
    }

    #[test]
    fn remove_peer() {
        let resolver = PeerResolver::new();
        let peer = make_peer(0x01);
        resolver.insert(peer, empty_resolved());

        let removed = resolver.remove(&peer);
        assert!(removed.is_some());
        assert!(resolver.get(&peer).is_none());
        assert_eq!(resolver.len(), 0);
    }

    #[test]
    fn concurrent_insert_no_duplicate() {
        use std::sync::Arc;
        let resolver = Arc::new(PeerResolver::with_capacity(100));
        let peer = make_peer(0x01);

        let handles: Vec<_> = (0..8).map(|_| {
            let resolver = Arc::clone(&resolver);
            std::thread::spawn(move || {
                resolver.insert(peer, empty_resolved());
            })
        }).collect();

        for h in handles { h.join().unwrap(); }

        assert_eq!(resolver.len(), 1,
            "concurrent inserts of same key must produce one entry");
    }

    #[test]
    fn concurrent_reads() {
        use std::sync::Arc;
        let resolver = Arc::new(PeerResolver::new());
        let peer = make_peer(0x01);
        resolver.insert(peer, ResolvedPeer {
            locators: None,
            display_name: Some("test".into()),
            trust_state: None,
        });

        let handles: Vec<_> = (0..8).map(|_| {
            let resolver = Arc::clone(&resolver);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    let _ = resolver.get(&peer);
                    let _ = resolver.len();
                }
            })
        }).collect();

        for h in handles { h.join().unwrap(); }
    }
}
