use super::*;

const NOW: u64 = 1_000_000_000;
const ALICE: Sender = [1; 32];
const BOB: Sender = [2; 32];

fn nonce(i: usize) -> [u8; 16] {
    let mut n = [0u8; 16];
    n[..8].copy_from_slice(&u64::try_from(i).unwrap().to_le_bytes());
    n
}

#[test]
fn a_copy_is_in_flight_until_its_first_settles_or_is_forgotten() {
    let mut guard = ReplayGuard::new();
    assert_eq!(guard.observe(&ALICE, &nonce(1), NOW, NOW), Sighting::First);
    assert_eq!(
        guard.observe(&ALICE, &nonce(1), NOW, NOW),
        Sighting::InFlight
    );
    guard.settle(&ALICE, &nonce(1));
    assert_eq!(
        guard.observe(&ALICE, &nonce(1), NOW, NOW),
        Sighting::Duplicate
    );
    assert_eq!(guard.observe(&ALICE, &nonce(2), NOW, NOW), Sighting::First);
    guard.forget(&ALICE, &nonce(2));
    assert_eq!(guard.observe(&ALICE, &nonce(2), NOW, NOW), Sighting::First);
}

#[test]
fn first_then_duplicate() {
    let mut guard = ReplayGuard::new();
    assert_eq!(guard.observe(&ALICE, &nonce(1), NOW, NOW), Sighting::First);
    guard.settle(&ALICE, &nonce(1));
    assert_eq!(
        guard.observe(&ALICE, &nonce(1), NOW + 5, NOW + 10),
        Sighting::Duplicate
    );
    assert_eq!(guard.observe(&ALICE, &nonce(2), NOW, NOW), Sighting::First);
    assert_eq!(guard.observe(&BOB, &nonce(1), NOW, NOW), Sighting::First);
}

#[test]
fn entries_expire_with_the_freshness_window() {
    let mut guard = ReplayGuard::new();
    guard.observe(&ALICE, &nonce(1), NOW, NOW);
    let after = NOW + ENVELOPE_FRESHNESS_WINDOW_MS + ENVELOPE_MAX_FUTURE_SKEW_MS + 1;
    guard.observe(&BOB, &nonce(1), after, after);
    assert_eq!(guard.len(), 1, "the expired entry is pruned");
    assert!(!guard.held.contains_key(&ALICE), "its share is released");
}

#[test]
fn a_sender_over_its_quota_blocks_no_one_else() {
    let mut guard = ReplayGuard::new();
    for i in 0..PER_SENDER_QUOTA {
        assert_eq!(guard.observe(&ALICE, &nonce(i), NOW, NOW), Sighting::First);
    }
    assert_eq!(
        guard.observe(&ALICE, &nonce(PER_SENDER_QUOTA), NOW, NOW),
        Sighting::Full
    );
    assert_eq!(
        guard.observe(&ALICE, &nonce(0), NOW, NOW),
        Sighting::InFlight,
        "live entries are never evicted early"
    );
    assert_eq!(guard.observe(&BOB, &nonce(0), NOW, NOW), Sighting::First);
    // A forgotten entry gives its share back.
    guard.forget(&ALICE, &nonce(0));
    assert_eq!(
        guard.observe(&ALICE, &nonce(PER_SENDER_QUOTA), NOW, NOW),
        Sighting::First
    );
}

#[test]
fn a_sender_spending_its_bucket_leaves_others_theirs() {
    let mut guard = ReplayGuard::new();
    for _ in 0..BUCKET_BURST {
        assert!(guard.may_open(&ALICE, NOW));
    }
    assert!(!guard.may_open(&ALICE, NOW));
    assert!(guard.may_open(&BOB, NOW));
    assert!(guard.may_open(&ALICE, NOW + BUCKET_REFILL_MS), "it refills");
}
