//! The member route cache (plan E3.4 S4, "Routes"): each member's verified
//! general route blob from their member record's `M`/6, by pseudonym, in one
//! community. Veilid-free; the Host keeps one per community in its
//! `CommunityState` beside the [`BeatView`](super::BeatView), on both modes,
//! and every Host reader of a member's route (MEK answering and
//! distribution, the key transfer, files, voice) reads it here.
//!
//! - **Fed by the record, never by gossip.** An entry is a value read from
//!   `M`/6 and verified under the member's pseudonym, delivered by a ring
//!   watch, a `HeadAdvanced` hint for subkey 6 or a pinned read.
//! - **Replaced, never aged.** A route is good exactly as long as its owner
//!   keeps it (Veilid replaces a dead route and the owner rewrites `M`/6,
//!   `rekindle-node` `member_routes_loop`), so an entry has no TTL: a newer
//!   seq replaces it, and a `RouteUnusable` from the transport forgets it
//!   ([`MemberRouteCache::forget`]) so the next use reads `M`/6 again.
//!   Veilid's own import cache is per blob (`RoutePort::import`), so this
//!   is not the transport's per-friend `PeerRegistry`.
//! - **Ordering.** `M`/6 carries the DHT seq of its write; a value at or
//!   below the seq held is an older copy (a lagging replica or a repeated
//!   hint) and changes nothing.

use std::collections::BTreeMap;

use rekindle_types::id::PseudonymKey;

/// One member's last verified route.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedRoute {
    blob: Vec<u8>,
    seq: u64,
}

/// The member route cache of one community, by member pseudonym (hex).
#[derive(Debug, Clone, Default)]
pub struct MemberRouteCache {
    by_member: BTreeMap<String, CachedRoute>,
}

impl MemberRouteCache {
    /// `member`'s cached route blob.
    #[must_use]
    pub fn get(&self, member: &PseudonymKey) -> Option<&[u8]> {
        self.by_member
            .get(&member.to_hex())
            .map(|r| r.blob.as_slice())
    }

    /// Take `blob`, `member`'s verified `M`/6 route at DHT `seq`; `true`
    /// when it replaced what was held (an older or equal seq changes
    /// nothing, an empty blob is no route).
    pub fn observe(&mut self, member: &PseudonymKey, seq: u64, blob: Vec<u8>) -> bool {
        if blob.is_empty() {
            return false;
        }
        let key = member.to_hex();
        if self.by_member.get(&key).is_some_and(|held| seq <= held.seq) {
            return false;
        }
        self.by_member.insert(key, CachedRoute { blob, seq });
        true
    }

    /// Forget `member`'s route (the transport said it is unusable); the
    /// blob that was held.
    pub fn forget(&mut self, member: &PseudonymKey) -> Option<Vec<u8>> {
        self.by_member.remove(&member.to_hex()).map(|r| r.blob)
    }

    /// Keep only the members `keep` accepts (a departed member's route
    /// goes with them).
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.by_member.retain(|member, _| keep(member));
    }

    /// The cached route of each member of `roster` that has one, in roster
    /// order (plan E3.4 S5's mesh peers: roster ∩ `M`/6).
    #[must_use]
    pub fn routes_for(&self, roster: &[PseudonymKey]) -> Vec<(PseudonymKey, Vec<u8>)> {
        roster
            .iter()
            .filter_map(|m| self.get(m).map(|blob| (m.clone(), blob.to_vec())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    #[test]
    fn a_newer_seq_replaces_and_an_older_or_equal_one_changes_nothing() {
        let mut cache = MemberRouteCache::default();
        assert!(cache.observe(&member(1), 3, b"A".to_vec()));
        assert!(
            !cache.observe(&member(1), 3, b"B".to_vec()),
            "a repeated seq"
        );
        assert!(
            !cache.observe(&member(1), 2, b"B".to_vec()),
            "a lagging replica"
        );
        assert_eq!(cache.get(&member(1)), Some(&b"A"[..]));
        assert!(cache.observe(&member(1), 4, b"B".to_vec()));
        assert_eq!(cache.get(&member(1)), Some(&b"B"[..]));
        assert!(!cache.observe(&member(2), 9, Vec::new()), "no route");
        assert_eq!(cache.get(&member(2)), None);
    }

    #[test]
    fn forgetting_a_route_takes_the_next_read_at_any_seq() {
        let mut cache = MemberRouteCache::default();
        cache.observe(&member(1), 5, b"A".to_vec());
        assert_eq!(cache.forget(&member(1)), Some(b"A".to_vec()));
        assert_eq!(cache.get(&member(1)), None);
        assert!(
            cache.observe(&member(1), 5, b"A".to_vec()),
            "re-read after forget"
        );
    }

    #[test]
    fn routes_for_a_roster_skips_members_without_a_route_and_retain_drops() {
        let mut cache = MemberRouteCache::default();
        cache.observe(&member(1), 1, b"A".to_vec());
        cache.observe(&member(3), 1, b"C".to_vec());
        let roster = [member(3), member(2), member(1)];
        assert_eq!(
            cache.routes_for(&roster),
            vec![(member(3), b"C".to_vec()), (member(1), b"A".to_vec())]
        );
        let gone = member(3).to_hex();
        cache.retain(|m| m != gone);
        assert_eq!(cache.routes_for(&roster), vec![(member(1), b"A".to_vec())]);
    }
}
