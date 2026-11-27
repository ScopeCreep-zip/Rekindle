use super::*;

#[test]
fn a_held_change_is_taken_once_by_its_author_and_message() {
    let mut held = HeldChanges::default();
    held.hold("AA", "m1", 1, 10);
    held.hold("aa", "m1", 2, 10);
    assert!(held.take("bb", "m1").is_empty(), "another author's");
    assert_eq!(held.take("aa", "m1"), vec![1, 2]);
    assert!(held.take("aa", "m1").is_empty());
    assert!(held.is_empty());
}

#[test]
fn the_key_bound_evicts_the_oldest_key() {
    let mut held = HeldChanges::default();
    for i in 0..=MAX_HELD_KEYS {
        held.hold("aa", &format!("m{i}"), i, 1);
    }
    assert_eq!(held.len(), MAX_HELD_KEYS);
    assert!(held.take("aa", "m0").is_empty(), "the oldest key went");
    assert_eq!(
        held.take("aa", &format!("m{MAX_HELD_KEYS}")),
        vec![MAX_HELD_KEYS]
    );
}

#[test]
fn the_byte_bound_evicts_the_oldest_key() {
    let mut held = HeldChanges::with_bounds(10, 100);
    held.hold("aa", "m1", 1, 60);
    held.hold("aa", "m2", 2, 60);
    assert!(
        held.take("aa", "m1").is_empty(),
        "past 100 bytes the oldest went"
    );
    assert_eq!(held.take("aa", "m2"), vec![2]);
    held.hold("aa", "m3", 3, 101);
    assert!(held.is_empty(), "a change past the bound alone is not held");
}
