//! The k = 4 watch ring (plan E3.2 "Watching", `evidence/slot-integrity.md`
//! §4.5): admitted members are ordered by
//! `blake3("rekindle-watch-ring-v1" ‖ communityId ‖ p)` and each watches
//! the member records (`M`) of its [`RING_K`] ring successors, so every `M`
//! has `min(k, N − 1)` watchers, under Veilid's `public_watch_limit = 32`
//! (`~/veilid/veilid-core/src/veilid_config.rs:921`). A member-set change
//! moves at most k assignments, since only the changed member's neighbours'
//! successor lists change.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;

/// Watchers per member record.
pub const RING_K: usize = 4;

/// The ring's hash domain.
pub const RING_DOMAIN: &[u8] = b"rekindle-watch-ring-v1";

/// `p`'s position on `community_id`'s ring. The domain and `p` are fixed
/// length, so the concatenation is unambiguous.
#[must_use]
pub fn ring_position(community_id: &str, p: &PseudonymKey) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RING_DOMAIN);
    hasher.update(community_id.as_bytes());
    hasher.update(&p.0);
    *hasher.finalize().as_bytes()
}

/// The member records `me` watches: its `k` ring successors among the
/// admitted members of `state`, nearest first. Empty while `me` is not
/// admitted.
#[must_use]
pub fn ring_successors(
    community_id: &str,
    state: &GovernanceState,
    me: &PseudonymKey,
    k: usize,
) -> Vec<String> {
    if !state.members.contains_key(me) {
        return Vec::new();
    }
    let mut ring: Vec<([u8; 32], &PseudonymKey, &str)> = state
        .members
        .iter()
        .map(|(p, m)| (ring_position(community_id, p), p, m.member_record.as_str()))
        .collect();
    ring.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1 .0.cmp(&b.1 .0)));
    let Some(at) = ring.iter().position(|(_, p, _)| *p == me) else {
        return Vec::new();
    };
    let n = ring.len();
    (1..n)
        .take(k)
        .map(|step| ring[(at + step) % n].2.to_owned())
        .collect()
}

#[cfg(test)]
#[path = "watch_ring_tests.rs"]
mod tests;
