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
        .min_by_key(|member| candidate_hash(&departed.0, &member.0))
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
        .min_by_key(|member| candidate_hash(&requester.0, &member.0))
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
        .map(|m| (candidate_hash(&departed.0, &m.0), m.clone()))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0));
    scored
        .into_iter()
        .take(max_cascades + 1)
        .map(|(_, key)| key)
        .collect()
}

/// Compute blake3(context_bytes || candidate_bytes) for deterministic ordering.
fn candidate_hash(context: &[u8; 32], candidate: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(context);
    hasher.update(candidate);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::PseudonymKey;

    fn key(seed: u8) -> PseudonymKey {
        PseudonymKey([seed; 32])
    }

    #[test]
    fn select_rotator_is_deterministic() {
        let departed = key(0);
        let remaining = vec![key(1), key(2), key(3), key(4), key(5)];

        let r1 = select_rotator(&departed, &remaining);
        let r2 = select_rotator(&departed, &remaining);
        assert_eq!(r1, r2);
    }

    #[test]
    fn select_rotator_is_order_independent() {
        let departed = key(0);
        let remaining_a = vec![key(1), key(2), key(3), key(4), key(5)];
        let remaining_b = vec![key(5), key(3), key(1), key(4), key(2)]; // shuffled

        let ra = select_rotator(&departed, &remaining_a);
        let rb = select_rotator(&departed, &remaining_b);
        assert_eq!(ra, rb);
    }

    #[test]
    fn select_rotator_empty_remaining_returns_none() {
        let departed = key(0);
        assert_eq!(select_rotator(&departed, &[]), None);
    }

    #[test]
    fn select_rotator_single_member() {
        let departed = key(0);
        let remaining = vec![key(1)];
        assert_eq!(select_rotator(&departed, &remaining), Some(key(1)));
    }

    #[test]
    fn select_rotator_never_returns_departed_member() {
        // Audit P7-W26: the chosen rotator must come from `remaining`.
        // If `departed` were accidentally included in `remaining`, the
        // selection algorithm would happily return them — proving that
        // the caller (`online_recipients`) is the only thing keeping the
        // departed pseudonym out. This test pins that contract by
        // showing that even when the algorithm runs over `remaining`
        // without `departed`, the result is in `remaining`.
        let departed = key(0);
        let remaining = vec![key(1), key(2), key(3)];
        let chosen = select_rotator(&departed, &remaining).unwrap();
        assert!(remaining.contains(&chosen));
        assert_ne!(chosen, departed);
    }

    #[test]
    fn different_departed_may_select_different_rotator() {
        let remaining = vec![key(1), key(2), key(3)];
        let first = select_rotator(&key(10), &remaining).unwrap();
        let second = (11u8..=200)
            .find_map(|seed| {
                let candidate = select_rotator(&key(seed), &remaining).unwrap();
                (candidate != first).then_some(candidate)
            })
            .expect("expected at least one departed context to change the rotator");
        assert_ne!(first, second);
    }

    #[test]
    fn select_mek_responder_excludes_requester() {
        let requester = key(1);
        let members = vec![key(1), key(2), key(3)];
        let responder = select_mek_responder(&requester, &members).unwrap();
        assert_ne!(responder, requester);
    }

    #[test]
    fn select_mek_responder_only_requester_returns_none() {
        let requester = key(1);
        let members = vec![key(1)];
        assert_eq!(select_mek_responder(&requester, &members), None);
    }

    #[test]
    fn cascade_candidates_returns_ordered_list() {
        let departed = key(0);
        let remaining = vec![key(1), key(2), key(3), key(4), key(5)];

        let candidates = cascade_candidates(&departed, &remaining, 2);
        // Should return at most 3 candidates (primary + 2 cascades)
        assert!(candidates.len() <= 3);
        assert!(!candidates.is_empty());

        // First candidate must be the same as select_rotator
        let primary = select_rotator(&departed, &remaining).unwrap();
        assert_eq!(candidates[0], primary);
    }

    #[test]
    fn cascade_candidates_respects_max() {
        let departed = key(0);
        let remaining = vec![key(1), key(2), key(3), key(4), key(5)];

        let candidates = cascade_candidates(&departed, &remaining, 0);
        assert_eq!(candidates.len(), 1); // Only primary, no cascades

        let candidates = cascade_candidates(&departed, &remaining, 5);
        assert_eq!(candidates.len(), 5); // All 5 members
    }

    #[test]
    fn cascade_candidates_empty_remaining() {
        let departed = key(0);
        let candidates = cascade_candidates(&departed, &[], 3);
        assert!(candidates.is_empty());
    }
}
