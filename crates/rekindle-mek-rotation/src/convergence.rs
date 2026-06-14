//! Deterministic same-generation MEK conflict resolution.
//!
//! Two peers can mint *different* random key bytes for the *same* generation
//! (e.g. the single-rotator election in [`crate::wait_for_rotation_slot`] races
//! when peers hold divergent presence views, or a cascade fallback fires while
//! the primary's transfer is still in flight). Without a tiebreak, each peer
//! keeps whichever `MekTransfer` arrived last → divergent key bytes at equal
//! generation → AEAD decrypt failures ("MEK decrypt failed at matching
//! generation"). This is the channel-MEK split-brain.
//!
//! The fix is convergent, deterministic, and coordinator-free: every minted key
//! carries its rotator's election rank (`blake3(election_context ||
//! minter_pseudonym)` — the same value the rotator election ranks by, exported
//! as `rekindle_secrets::rotator::election_hash`). The canonical key for a
//! generation is the one with the lexicographically lowest rank — i.e. the key
//! minted by the rightful primary rotator. Every peer applies the same pure
//! comparison and converges on the same key regardless of arrival order, with
//! no coordinator and no privileged node.
//!
//! Forward secrecy is preserved: keys are still random per rotation, and a
//! departed member receives none of the candidate keys, so converging on the
//! lowest-rank key reveals nothing to them.

/// Decide whether an `incoming` MEK should REPLACE the `cached` MEK when both
/// are at the SAME generation. Compares the minters' election ranks:
///
/// - both tagged → the lower rank wins (the rightful primary rotator),
/// - cached untagged, incoming tagged → incoming wins (a real rotator's key
///   beats a legacy/untagged one),
/// - incoming untagged, or both untagged, or equal → keep cached (idempotent;
///   never thrash on a re-delivery of the same key).
///
/// The relation is a deterministic total order on the 32-byte ranks, so
/// cross-delivery in either order converges to the identical key on every peer.
#[must_use]
pub fn incoming_wins_same_generation(
    cached_rank: Option<&[u8; 32]>,
    incoming_rank: Option<&[u8; 32]>,
) -> bool {
    match (cached_rank, incoming_rank) {
        (Some(cached), Some(incoming)) => incoming < cached,
        (None, Some(_)) => true,
        (Some(_) | None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::incoming_wins_same_generation;

    const LOW: [u8; 32] = [0x10; 32];
    const HIGH: [u8; 32] = [0x20; 32];

    #[test]
    fn lower_rank_wins() {
        // cached=HIGH, incoming=LOW → incoming (lower) replaces.
        assert!(incoming_wins_same_generation(Some(&HIGH), Some(&LOW)));
        // cached=LOW, incoming=HIGH → keep cached.
        assert!(!incoming_wins_same_generation(Some(&LOW), Some(&HIGH)));
    }

    #[test]
    fn equal_rank_is_idempotent() {
        assert!(!incoming_wins_same_generation(Some(&LOW), Some(&LOW)));
    }

    #[test]
    fn tagged_beats_untagged() {
        assert!(incoming_wins_same_generation(None, Some(&HIGH)));
    }

    #[test]
    fn untagged_never_replaces() {
        assert!(!incoming_wins_same_generation(Some(&LOW), None));
        assert!(!incoming_wins_same_generation(None, None));
    }

    #[test]
    fn converges_regardless_of_arrival_order() {
        // Whichever order the two same-gen keys arrive, the cache must end up
        // holding the LOW-rank key. Simulate both orders.
        // Order A: cache LOW first, then HIGH arrives.
        let mut cached: Option<[u8; 32]> = Some(LOW);
        if incoming_wins_same_generation(cached.as_ref(), Some(&HIGH)) {
            cached = Some(HIGH);
        }
        assert_eq!(cached, Some(LOW));

        // Order B: cache HIGH first, then LOW arrives.
        let mut cached: Option<[u8; 32]> = Some(HIGH);
        if incoming_wins_same_generation(cached.as_ref(), Some(&LOW)) {
            cached = Some(LOW);
        }
        assert_eq!(cached, Some(LOW));
    }
}
