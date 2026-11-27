use super::{OsRandom, RandomSource};

#[test]
fn below_stays_in_range() {
    for bound in [1u64, 2, 3, 121, 300] {
        for _ in 0..200 {
            assert!(OsRandom.below(bound) < bound);
        }
    }
    assert_eq!(OsRandom.below(0), 0);
}
