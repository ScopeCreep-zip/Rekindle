//! The shared frontier reads and the once-only divergence memo.

use super::*;

#[test]
fn a_clone_shares_the_reads() {
    let sweep = SeenFrontiers::new();
    let ring = sweep.clone();
    assert!(ring.mark_read("VLD0:m", 5));
    assert_eq!(sweep.get("VLD0:m"), Some(5));
    assert!(
        !sweep.unread("VLD0:m", 5),
        "the ring's read dedupes the sweep"
    );
    assert!(sweep.unread("VLD0:m", 6));
    assert!(!sweep.mark_read("VLD0:m", 4), "a lower seq moves nothing");
    assert_eq!(ring.get("VLD0:m"), Some(5));
}

#[test]
fn a_divergence_is_noted_once_per_heads_and_checksum() {
    let seen = SeenFrontiers::new();
    let heads = op_set_digest(&[OpHash([1; 32])]);
    assert!(seen.note_divergence("VLD0:m", heads, RosterChecksum([2; 32])));
    assert!(!seen.note_divergence("VLD0:m", heads, RosterChecksum([2; 32])));
    assert!(seen.note_divergence("VLD0:m", heads, RosterChecksum([3; 32])));
}

#[test]
fn the_op_set_digest_ignores_order() {
    let a = OpHash([1; 32]);
    let b = OpHash([2; 32]);
    assert_eq!(op_set_digest(&[a, b]), op_set_digest(&[b, a]));
    assert_ne!(op_set_digest(&[a]), op_set_digest(&[a, b]));
}
