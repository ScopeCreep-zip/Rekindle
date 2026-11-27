use super::*;

#[test]
fn an_export_carries_no_device_scoped_entry_and_the_live_vault_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alice.vault");
    let (vault, _) = VaultStore::create(&path, "pp").unwrap();
    let op_key = VaultKey::DeviceOpKey {
        community: "c1".into(),
    };
    vault.put(&VaultKey::IdentityEd25519, &[3u8; 32]).unwrap();
    vault.put(&VaultKey::DeviceVaultSecret, &[7u8; 32]).unwrap();
    vault.put(&op_key, b"device key").unwrap();
    let (bytes, header) = vault.export_files().unwrap();

    let copy = dir.path().join("b/alice.vault");
    VaultStore::import_files(&copy, &bytes, &header).unwrap();
    let (opened, _) = VaultStore::open(&copy, "pp").unwrap();
    assert_eq!(opened.get(&VaultKey::DeviceVaultSecret).unwrap(), None);
    assert_eq!(opened.get(&op_key).unwrap(), None);
    assert_eq!(
        &*opened.get(&VaultKey::IdentityEd25519).unwrap().unwrap(),
        &[3u8; 32]
    );
    // The live vault is untouched and still writable.
    assert_eq!(
        &*vault.get(&VaultKey::DeviceVaultSecret).unwrap().unwrap(),
        &[7u8; 32]
    );
    assert_eq!(&*vault.get(&op_key).unwrap().unwrap(), b"device key");
    vault.put(&VaultKey::AuditTail, b"after").unwrap();
    // No scratch copy or journal is left beside the vault.
    let mut left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["alice.vault", "alice.vault.hdr", "b"]);
}
