use super::*;

#[test]
fn allocate_unique_role_id_returns_unused() {
    let existing = vec![RoleId([1; 16]), RoleId([2; 16]), RoleId([3; 16])];
    let id = allocate_unique_role_id(&existing, &rekindle_utils::OsRandom).expect("found");
    assert!(!existing.contains(&id));
}

/// A new role is never `@everyone`, whatever the source draws.
#[test]
fn allocate_unique_role_id_never_hands_out_everyone() {
    struct Zeros;
    impl rand::CryptoRng for Zeros {}
    impl rekindle_utils::RandomSource for Zeros {
        fn fill(&self, dest: &mut [u8]) {
            dest.fill(0);
        }
    }
    assert_eq!(allocate_unique_role_id(&[], &Zeros), None);
}

#[test]
fn hex_to_pseudo_zero_on_bad_hex() {
    assert_eq!(PseudonymKey::from_hex_lossy("not hex").0, [0u8; 32]);
}
