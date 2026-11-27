use super::*;
use rekindle_types::governance::OpHash;

fn joined(lamport: u64) -> OpStamp {
    OpStamp {
        lamport,
        hash: OpHash([7; 32]),
    }
}

#[test]
fn a_copy_after_the_rotation_landed_is_spent() {
    let mut rotated = RotatedDepartures::default();
    rotated.record("c", "AB", None, 1);
    assert!(rotated.is_spent("c", "ab", None, 2));
    assert!(!rotated.is_spent("other", "ab", None, 2));
}

#[test]
fn a_copy_seen_before_the_removal_merged_matches_one_seen_after() {
    let mut rotated = RotatedDepartures::default();
    rotated.record("c", "ab", Some(joined(3)), 1);
    assert!(rotated.is_spent("c", "ab", None, 2));
    assert!(rotated.is_spent("c", "ab", Some(joined(3)), 2));
}

#[test]
fn a_departure_nobody_rotated_for_is_still_owed() {
    let mut rotated = RotatedDepartures::default();
    rotated.record("c", "ab", None, 1);
    assert!(!rotated.is_spent("c", "ab", None, 1));
}

#[test]
fn a_readmitted_member_departs_anew() {
    let mut rotated = RotatedDepartures::default();
    rotated.record("c", "ab", None, 1);
    assert!(!rotated.is_spent("c", "ab", Some(joined(9)), 2));
    rotated.record("c", "ab", Some(joined(3)), 1);
    assert!(!rotated.is_spent("c", "ab", Some(joined(9)), 2));
}
