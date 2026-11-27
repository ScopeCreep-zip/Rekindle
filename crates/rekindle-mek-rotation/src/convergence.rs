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
#[path = "convergence_tests.rs"]
mod tests;
