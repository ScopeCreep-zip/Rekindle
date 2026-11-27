use super::waits_for_key;

/// Generations 2 and 3 held, the state converged on 4.
fn held(g: u64) -> bool {
    (2..=3).contains(&g)
}

#[test]
fn a_converged_generation_not_held_yet_waits() {
    assert!(waits_for_key(4, Some(2), 4, held));
}

#[test]
fn before_any_key_every_entry_waits() {
    assert!(waits_for_key(1, None, 4, held));
}

/// A private channel's generation above every converged community
/// bump (a channel we were removed from) never holds the cursor.
#[test]
fn a_generation_no_converged_bump_owes_moves_on() {
    assert!(!waits_for_key(7, Some(2), 4, held));
}

#[test]
fn a_generation_before_our_first_key_moves_on() {
    assert!(!waits_for_key(1, Some(2), 4, held));
}

#[test]
fn a_held_generation_whose_tag_maps_nowhere_moves_on() {
    assert!(!waits_for_key(3, Some(2), 4, held));
}
