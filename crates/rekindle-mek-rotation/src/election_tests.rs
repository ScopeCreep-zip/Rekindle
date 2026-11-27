use super::*;
use rekindle_types::id::PseudonymKey;

fn pk(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

#[test]
fn cascade_delay_level_zero_is_immediate() {
    assert_eq!(cascade_delay(0), Duration::ZERO);
}

#[test]
fn cascade_delay_increments_by_thirty_seconds() {
    assert_eq!(cascade_delay(1), Duration::from_secs(30));
    assert_eq!(cascade_delay(2), Duration::from_secs(60));
    assert_eq!(cascade_delay(3), Duration::from_secs(90));
}

#[test]
fn max_cascades_is_three() {
    assert_eq!(MAX_CASCADES, 3);
}

#[test]
fn select_mek_responder_excludes_requester() {
    let requester = pk(1);
    let members = vec![pk(1), pk(2), pk(3)];
    let selected = select_mek_responder(&requester, &members).expect("at least one candidate");
    assert_ne!(selected, requester);
}

#[test]
fn select_mek_responder_returns_none_when_only_requester() {
    let requester = pk(1);
    let members = vec![pk(1)];
    assert!(select_mek_responder(&requester, &members).is_none());
}

#[test]
fn select_mek_responder_deterministic_across_calls() {
    let requester = pk(7);
    let members = vec![pk(2), pk(3), pk(4), pk(5)];
    let a = select_mek_responder(&requester, &members).unwrap();
    let b = select_mek_responder(&requester, &members).unwrap();
    assert_eq!(a, b);
}

#[test]
fn cascade_candidates_returns_max_plus_one() {
    let departed = pk(0);
    let remaining = vec![pk(2), pk(3), pk(4), pk(5), pk(6)];
    let cascade = cascade_candidates(&departed, &remaining, MAX_CASCADES);
    // max_cascades=3 → at most 4 candidates (primary + 3 fallbacks)
    assert!(cascade.len() <= MAX_CASCADES + 1);
    assert!(!cascade.is_empty());
}

#[test]
fn cascade_candidates_returns_subset_when_fewer_remaining() {
    let departed = pk(0);
    let remaining = vec![pk(2), pk(3)];
    let cascade = cascade_candidates(&departed, &remaining, MAX_CASCADES);
    assert_eq!(cascade.len(), 2);
}

#[test]
fn cascade_candidates_empty_when_no_remaining() {
    let departed = pk(0);
    let cascade = cascade_candidates(&departed, &[], MAX_CASCADES);
    assert!(cascade.is_empty());
}

#[test]
fn cascade_candidates_ordering_is_deterministic() {
    let departed = pk(42);
    let remaining = vec![pk(10), pk(20), pk(30), pk(40)];
    let a = cascade_candidates(&departed, &remaining, MAX_CASCADES);
    let b = cascade_candidates(&departed, &remaining, MAX_CASCADES);
    assert_eq!(a, b);
}
