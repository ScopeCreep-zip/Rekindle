//! Deterministic rotator and responder selection for peer MEK distribution.
//!
//! When a member departs a community, the MEK must rotate so the departed
//! member can't decrypt future messages. Since there is no coordinator,
//! a **deterministic rotator** is elected by all peers independently —
//! same algorithm, same inputs, same result.
//!
//! Algorithm: For each candidate, compute `blake3(context || candidate)`.
//! The candidate with the lexicographically lowest hash is selected.
//! This is identical on every peer because the inputs are deterministic.
//!
//! Zero async, zero I/O — pure function suitable for library crate.

use rekindle_types::id::PseudonymKey;

/// Select the deterministic MEK rotator after a member departs.
///
/// Every peer computes this independently and arrives at the same result.
/// The `departed` key is the context — the member whose departure triggered
/// rotation. `remaining` is the set of members still in the community.
///
/// Returns `None` if `remaining` is empty.
pub fn select_rotator(departed: &PseudonymKey, remaining: &[PseudonymKey]) -> Option<PseudonymKey> {
    remaining
        .iter()
        .min_by_key(|member| election_hash(&departed.0, &member.0))
        .cloned()
}

/// Select the deterministic MEK responder for a `RequestMEK` message.
///
/// When a peer misses a MEK delivery (was offline during rotation), they
/// broadcast `RequestMEK`. To prevent N peers from all responding (flood),
/// only the deterministic responder replies.
///
/// The requester is excluded from candidates.
///
/// Returns `None` if no candidates remain after excluding the requester.
pub fn select_mek_responder(
    requester: &PseudonymKey,
    members: &[PseudonymKey],
) -> Option<PseudonymKey> {
    members
        .iter()
        .filter(|m| m != &requester)
        .min_by_key(|member| election_hash(&requester.0, &member.0))
        .cloned()
}

/// Return up to `max_cascades + 1` candidates sorted by their hash.
///
/// The first candidate is the primary rotator. If it fails to deliver
/// within the cascade timeout (30s), the next candidate takes over, and
/// so on up to `max_cascades` fallback levels.
pub fn cascade_candidates(
    departed: &PseudonymKey,
    remaining: &[PseudonymKey],
    max_cascades: usize,
) -> Vec<PseudonymKey> {
    let mut scored: Vec<_> = remaining
        .iter()
        .map(|m| (election_hash(&departed.0, &m.0), m.clone()))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0));
    scored
        .into_iter()
        .take(max_cascades + 1)
        .map(|(_, key)| key)
        .collect()
}

/// Compute `blake3(context || candidate)` — the deterministic election rank.
///
/// This is the single source of the ordering used by [`select_rotator`],
/// [`select_mek_responder`], and [`cascade_candidates`]. It is also the rank a
/// minting rotator stamps onto its MEK (`MediaEncryptionKey::with_provenance`)
/// so every peer can deterministically converge on the lowest-rank (= rightful
/// primary) key for a generation, resolving same-generation split-brain.
#[must_use]
pub fn election_hash(context: &[u8; 32], candidate: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(context);
    hasher.update(candidate);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
#[path = "rotator_tests.rs"]
mod tests;
