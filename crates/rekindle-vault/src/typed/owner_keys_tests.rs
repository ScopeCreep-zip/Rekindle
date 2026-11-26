use super::*;

fn entry(key: &str, keypair: &str) -> OwnerEntry {
    OwnerEntry {
        record_key: key.into(),
        keypair: keypair.into(),
    }
}

#[test]
fn each_record_keeps_its_own_entry() {
    let dir = tempfile::TempDir::new().unwrap();
    let (vault, _) = VaultStore::create(
        &dir.path().join("a.vault"),
        rekindle_test_fixtures::PASSPHRASE,
    )
    .unwrap();
    assert_eq!(load_owner(&vault, OwnerKey::Profile).unwrap(), None);
    persist_owner(&vault, OwnerKey::Profile, &entry("k1", "kp1")).unwrap();
    persist_owner(&vault, OwnerKey::Account, &entry("k2", "kp2")).unwrap();
    persist_owner(&vault, OwnerKey::Profile, &entry("k3", "kp3")).unwrap();
    assert_eq!(
        load_owner(&vault, OwnerKey::Profile).unwrap(),
        Some(entry("k3", "kp3"))
    );
    assert_eq!(
        load_owner(&vault, OwnerKey::Account).unwrap(),
        Some(entry("k2", "kp2"))
    );
    assert_eq!(load_owner(&vault, OwnerKey::FriendList).unwrap(), None);
    forget_owner(&vault, OwnerKey::Account).unwrap();
    assert_eq!(load_owner(&vault, OwnerKey::Account).unwrap(), None);
}

#[test]
fn a_bare_keypair_is_not_an_entry() {
    let dir = tempfile::TempDir::new().unwrap();
    let (vault, _) = VaultStore::create(
        &dir.path().join("a.vault"),
        rekindle_test_fixtures::PASSPHRASE,
    )
    .unwrap();
    vault.put(&VaultKey::ProfileOwnerKeypair, b"kp").unwrap();
    assert!(load_owner(&vault, OwnerKey::Profile).is_err());
}

#[test]
fn the_debug_form_hides_the_keypair() {
    assert!(!format!("{:?}", entry("k", "secret-kp")).contains("secret-kp"));
}
