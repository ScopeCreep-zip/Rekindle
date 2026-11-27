use super::*;

fn op_key(community: &str) -> VaultKey {
    VaultKey::DeviceOpKey {
        community: community.into(),
    }
}

#[test]
fn the_device_vault_opens_after_the_identity_vault_with_one_argon2() {
    let dir = tempfile::tempdir().unwrap();
    let (path, device) = (
        dir.path().join("a.vault"),
        dir.path().join("a.device.vault"),
    );
    let (identity, _) = VaultStore::create(&path, rekindle_test_fixtures::PASSPHRASE).unwrap();
    let first = open(&identity, &device).unwrap();
    first.put(&op_key("c1"), b"device key").unwrap();
    drop(first);
    drop(identity);
    // One passphrase stretch: the identity vault's. The device vault
    // has no header (no Argon2 parameters) and opens from the key the
    // identity vault holds.
    assert!(!crate::header::header_path(&device).exists());
    let (identity, _) = VaultStore::open(&path, rekindle_test_fixtures::PASSPHRASE).unwrap();
    let again = open(&identity, &device).unwrap();
    assert_eq!(
        again.get(&op_key("c1")).unwrap().unwrap().as_slice(),
        b"device key"
    );
    // Another identity's vault holds no key to it.
    let (other, _) = VaultStore::create(
        &dir.path().join("b.vault"),
        rekindle_test_fixtures::PASSPHRASE,
    )
    .unwrap();
    assert!(matches!(
        open(&other, &device),
        Err(VaultError::WrongPassphrase)
    ));
}

#[test]
fn an_identity_export_never_carries_the_device_vault() {
    let dir = tempfile::tempdir().unwrap();
    let (path, device) = (
        dir.path().join("a.vault"),
        dir.path().join("a.device.vault"),
    );
    let (identity, _) = VaultStore::create(&path, rekindle_test_fixtures::PASSPHRASE).unwrap();
    let held = open(&identity, &device).unwrap();
    held.put(&op_key("c1"), b"device key").unwrap();
    let (vault_bytes, header_bytes) = identity.export_files().unwrap();
    // The export's vault holds no device secret.
    let imported = dir.path().join("b/a.vault");
    VaultStore::import_files(&imported, &vault_bytes, &header_bytes).unwrap();
    let (copy, _) = VaultStore::open(&imported, rekindle_test_fixtures::PASSPHRASE).unwrap();
    assert!(copy.list("device").unwrap().is_empty());
    // Nor the key to the exporting device's vault (ADR 0022 rule 4).
    assert_eq!(copy.get(&VaultKey::DeviceVaultSecret).unwrap(), None);
    assert_eq!(copy.get(&op_key("c1")).unwrap(), None);
    // The imported identity has no device vault: it mints its own, under
    // a fresh key, holding nothing of the exporting device's.
    let imported_device = dir.path().join("b/a.device.vault");
    let fresh = open(&copy, &imported_device).unwrap();
    assert_eq!(fresh.get(&op_key("c1")).unwrap(), None);
    assert_ne!(
        copy.get(&VaultKey::DeviceVaultSecret).unwrap(),
        identity.get(&VaultKey::DeviceVaultSecret).unwrap()
    );
    // The exporting device still opens its own.
    let own = open(&identity, &device).unwrap();
    assert_eq!(
        own.get(&op_key("c1")).unwrap().unwrap().as_slice(),
        b"device key"
    );
}
