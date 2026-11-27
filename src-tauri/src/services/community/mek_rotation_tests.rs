use super::cascade_cache_hit;

const KEY_A: [u8; 32] = [1u8; 32];
const KEY_B: [u8; 32] = [2u8; 32];

#[test]
fn split_brain_same_generation_different_key_is_not_a_hit_until_key_changes() {
    // The bug this guards against: we already hold generation 5
    // (KEY_A) and it's failing to decrypt — `needed_generation == 5`
    // looks "already satisfied" by generation alone. Re-resolving
    // the SAME key must not end the retry.
    assert!(!cascade_cache_hit(
        5,
        true,
        Some((KEY_A, 5)),
        Some((KEY_A, 5))
    ));
    // A different key landing at the same (or any) generation does.
    assert!(cascade_cache_hit(
        5,
        true,
        Some((KEY_A, 5)),
        Some((KEY_B, 5))
    ));
    assert!(cascade_cache_hit(
        5,
        true,
        Some((KEY_A, 5)),
        Some((KEY_B, 9))
    ));
    // Nothing resolved at all is never a hit.
    assert!(!cascade_cache_hit(5, true, Some((KEY_A, 5)), None));
}

#[test]
fn needed_generation_zero_wants_any_change_from_initial() {
    // "send me your current" (needed_generation == 0) is satisfied
    // by ANY resolution that differs from what we started with —
    // never by re-resolving to the same thing, even the same key.
    assert!(!cascade_cache_hit(
        0,
        false,
        Some((KEY_A, 1)),
        Some((KEY_A, 1))
    ));
    assert!(cascade_cache_hit(
        0,
        false,
        Some((KEY_A, 1)),
        Some((KEY_A, 2))
    ));
    assert!(cascade_cache_hit(0, false, None, Some((KEY_A, 1))));
    assert!(!cascade_cache_hit(0, false, None, None));
}

#[test]
fn explicit_generation_wants_at_or_above_needed() {
    assert!(cascade_cache_hit(5, false, None, Some((KEY_A, 5))));
    assert!(cascade_cache_hit(5, false, None, Some((KEY_A, 9))));
    assert!(!cascade_cache_hit(5, false, None, Some((KEY_A, 4))));
    assert!(!cascade_cache_hit(5, false, None, None));
}
