//! ResidentIdentity and ResidentSet — co-resident identity management.
//!
//! `ResidentSet` is the node-wide map of all identities hosted locally.
//! Design point: 100K–200K residents. DashMap-sharded, lock-free reads.
//!
//! Memory budget (normative ceiling): a `ResidentIdentity` with empty
//! caches ≤ 512 bytes heap-resident. 200K residents ≤ 256 MiB before
//! persona caches.

use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;

use crate::locator::record::LocatorRecord;
use crate::locator::GovernanceKey;
use crate::origin::originate::{IdentityRoot, RotationEpoch};
use crate::projection::pseudonym::CommunityPersona;
use crate::root::PeerRef;

/// One co-resident identity hosted by this node.
///
/// Hot, immutable-after-construction core behind `Arc`; mutable
/// satellites behind `ArcSwap`. Secrets are NOT here — they stay in
/// the per-identity vault, loaded on demand.
pub struct ResidentIdentity {
    /// The peer identity.
    pub peer: PeerRef,
    /// The current rotation epoch.
    pub epoch: RotationEpoch,
    /// The latest published locator record (hot-swappable).
    locators: ArcSwap<Option<LocatorRecord>>,
    /// Per-community persona cache (derived on demand, cached for O(1)).
    persona_cache: DashMap<GovernanceKeyHash, CommunityPersona>,
}

/// Hash wrapper for GovernanceKey to use as DashMap key without
/// carrying the full governance key (which contains a Vec).
#[derive(Clone, PartialEq, Eq, Hash)]
struct GovernanceKeyHash([u8; 32]);

impl GovernanceKeyHash {
    fn from_gov(gov: &GovernanceKey) -> Self {
        Self(*blake3::hash(gov.canonical_bytes()).as_bytes())
    }
}

impl ResidentIdentity {
    /// Construct a new resident identity.
    pub fn new(root: IdentityRoot, epoch: RotationEpoch) -> Self {
        Self {
            peer: PeerRef::anchor(root),
            epoch,
            locators: ArcSwap::new(Arc::new(None)),
            persona_cache: DashMap::new(),
        }
    }

    /// Update the published locator record (atomic swap).
    pub fn update_locators(&self, record: LocatorRecord) {
        self.locators.store(Arc::new(Some(record)));
    }

    /// Get the current locator record.
    pub fn locators(&self) -> Arc<Option<LocatorRecord>> {
        self.locators.load_full()
    }

    /// Cache a community persona for O(1) lookup.
    pub fn cache_persona(&self, gov: &GovernanceKey, persona: CommunityPersona) {
        self.persona_cache.insert(GovernanceKeyHash::from_gov(gov), persona);
    }

    /// Look up a cached community persona.
    pub fn cached_persona(&self, gov: &GovernanceKey) -> Option<CommunityPersona> {
        self.persona_cache.get(&GovernanceKeyHash::from_gov(gov)).map(|e| e.clone())
    }

    /// Number of cached community personas.
    pub fn persona_cache_size(&self) -> usize {
        self.persona_cache.len()
    }
}

impl core::fmt::Debug for ResidentIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResidentIdentity")
            .field("peer", &self.peer)
            .field("epoch", &self.epoch)
            .field("persona_cache_size", &self.persona_cache.len())
            .finish()
    }
}

/// The node-wide set of co-resident identities.
///
/// DashMap<PeerRef, Arc<ResidentIdentity>>. Lock-free reads via
/// DashMap shard read locks (no contention under read-heavy workloads).
pub struct ResidentSet {
    inner: DashMap<PeerRef, Arc<ResidentIdentity>>,
}

impl ResidentSet {
    /// Create an empty set.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Insert a new resident identity. Returns the Arc'd identity.
    pub fn insert(&self, resident: ResidentIdentity) -> Arc<ResidentIdentity> {
        let arc = Arc::new(resident);
        let peer = arc.peer;
        self.inner.insert(peer, Arc::clone(&arc));
        arc
    }

    /// Look up a resident by PeerRef (lock-free read).
    pub fn get(&self, peer: &PeerRef) -> Option<Arc<ResidentIdentity>> {
        self.inner.get(peer).map(|entry| Arc::clone(entry.value()))
    }

    /// Remove a resident (identity death).
    pub fn remove(&self, peer: &PeerRef) -> Option<Arc<ResidentIdentity>> {
        self.inner.remove(peer).map(|(_, arc)| arc)
    }

    /// Number of residents.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate all peer refs (for diagnostics).
    pub fn peer_refs(&self) -> Vec<PeerRef> {
        self.inner.iter().map(|entry| *entry.key()).collect()
    }
}

impl Default for ResidentSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::originate_from_seed;
    use crate::origin::seed::OriginSeed;
    use zeroize::Zeroizing;

    fn make_resident(byte: u8) -> ResidentIdentity {
        let o = originate_from_seed(
            OriginSeed::from_vault_bytes(Zeroizing::new([byte; 32]))
        ).unwrap();
        ResidentIdentity::new(o.root, RotationEpoch::ORIGIN)
    }

    #[test]
    fn insert_and_get() {
        let set = ResidentSet::new();
        let resident = make_resident(0x01);
        let peer = resident.peer;
        set.insert(resident);

        let found = set.get(&peer);
        assert!(found.is_some());
        assert_eq!(found.unwrap().peer, peer);
    }

    #[test]
    fn remove_returns_identity() {
        let set = ResidentSet::new();
        let resident = make_resident(0x01);
        let peer = resident.peer;
        set.insert(resident);

        let removed = set.remove(&peer);
        assert!(removed.is_some());
        assert!(set.get(&peer).is_none());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn persona_cache() {
        let resident = make_resident(0x01);
        let gov = GovernanceKey::parse("VLD0:test").unwrap();

        let persona = CommunityPersona {
            pseudonym: crate::projection::pseudonym::Pseudonym([0xAA; 32]),
            community_dh: crate::operational::dh::DhKey::from_bytes([0xBB; 32]),
            slot_index: 5,
            governance: gov.clone(),
        };

        assert!(resident.cached_persona(&gov).is_none());
        resident.cache_persona(&gov, persona.clone());
        assert_eq!(resident.cached_persona(&gov).unwrap().slot_index, 5);
        assert_eq!(resident.persona_cache_size(), 1);
    }

    #[test]
    fn concurrent_get() {
        use std::sync::Arc as StdArc;
        let set = StdArc::new(ResidentSet::new());
        let resident = make_resident(0x01);
        let peer = resident.peer;
        set.insert(resident);

        let handles: Vec<_> = (0..8).map(|_| {
            let set = StdArc::clone(&set);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    let _ = set.get(&peer);
                    let _ = set.len();
                }
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn many_residents() {
        let set = ResidentSet::new();
        for byte in 0..=255u8 {
            set.insert(make_resident(byte));
        }
        assert_eq!(set.len(), 256);
    }
}
