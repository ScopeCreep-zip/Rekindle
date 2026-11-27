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
