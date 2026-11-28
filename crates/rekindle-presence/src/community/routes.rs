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
use tokio::time::Instant;

use super::reread_cap::RereadCap;

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
    /// D8's cap on re-reads that found no usable route.
    rereads: RereadCap,
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
        self.rereads.observed(&key, seq);
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

    /// Forget every member's route whose blob `dead` names (the transport
    /// knows it dead, plan RV1g B3); how many.
    pub fn forget_blobs(&mut self, mut dead: impl FnMut(&[u8]) -> bool) -> usize {
        let before = self.by_member.len();
        self.by_member.retain(|_, r| !dead(&r.blob));
        before - self.by_member.len()
    }

    /// Keep only the members `keep` accepts (a departed member's route
    /// goes with them).
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.by_member.retain(|member, _| keep(member));
        self.rereads.retain(keep);
    }

    /// Whether a re-read of `member`'s `M`/6 after a failed send may go to
    /// the network at `now` (D8's cap, [`super::reread_cap`]).
    #[must_use]
    pub fn may_reread(&self, member: &PseudonymKey, now: Instant) -> bool {
        self.rereads.may_reread(&member.to_hex(), now)
    }

    /// A re-read of `member`'s `M`/6 after a send to `failed` failed read
    /// `read` (seq, blob; `None`: nothing) at `now`. The route to send to:
    /// the blob read, unless it is empty, the one that failed, or one
    /// `dead` names (the transport knows it dead, plan RV1g B1). Without
    /// one, the next re-read waits out D8's window.
    pub fn reread(
        &mut self,
        member: &PseudonymKey,
        failed: Option<&[u8]>,
        read: Option<(u64, Vec<u8>)>,
        dead: impl FnOnce(&[u8]) -> bool,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let seq = read.as_ref().map(|(seq, _)| *seq);
        let fresh = read
            .map(|(_, blob)| blob)
            .filter(|blob| !blob.is_empty() && Some(blob.as_slice()) != failed && !dead(blob));
        self.rereads
            .reread(&member.to_hex(), seq, fresh.is_some(), now);
        fresh
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
#[path = "routes_reread_tests.rs"]
mod reread_tests;

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
