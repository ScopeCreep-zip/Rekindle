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
