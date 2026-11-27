use super::*;

#[test]
fn the_journal_round_trips_and_clears() {
    let dir = tempfile::TempDir::new().unwrap();
    let (vault, _) = VaultStore::create(
        &dir.path().join("a.vault"),
        rekindle_test_fixtures::PASSPHRASE,
    )
    .unwrap();
    assert!(load(&vault).unwrap().is_none());
    let pending = PendingRotation {
        secret: Zeroizing::new([7; 32]),
        mint_seed: Zeroizing::new([9; 32]),
        issued_at_ms: 1234,
    };
    persist(&vault, &pending).unwrap();
    let back = load(&vault).unwrap().unwrap();
    assert_eq!(*back.secret, [7; 32]);
    assert_eq!(*back.mint_seed, [9; 32]);
    assert_eq!(back.issued_at_ms, 1234);
    clear(&vault).unwrap();
    assert!(load(&vault).unwrap().is_none());
}
