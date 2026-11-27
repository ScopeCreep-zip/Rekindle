use super::*;
use tempfile::TempDir;

fn fresh_path() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("alice.vault");
    (dir, path)
}

#[test]
fn exported_files_import_and_open_with_the_same_passphrase() {
    let (dir, path) = fresh_path();
    let (vault, key) = VaultStore::create(&path, "pp").unwrap();
    vault.put(&VaultKey::IdentityEd25519, &[3u8; 32]).unwrap();
    let (bytes, header) = vault.export_files().unwrap();
    let copy = dir.path().join("copy.vault");
    VaultStore::import_files(&copy, &bytes, &header).unwrap();
    assert!(matches!(
        VaultStore::import_files(&copy, &bytes, &header),
        Err(VaultError::AlreadyExists)
    ));
    let (opened, copy_key) = VaultStore::open(&copy, "pp").unwrap();
    assert_eq!(copy_key.as_bytes(), key.as_bytes());
    assert_eq!(
        &*opened.get(&VaultKey::IdentityEd25519).unwrap().unwrap(),
        &[3u8; 32]
    );
    assert!(matches!(
        VaultStore::import_files(&dir.path().join("bad.vault"), &bytes, b"junk"),
        Err(VaultError::Schema(_))
    ));
}

#[test]
fn create_then_open_and_open_never_creates() {
    let (_dir, path) = fresh_path();
    assert!(matches!(
        VaultStore::open(&path, "pp"),
        Err(VaultError::NotFound)
    ));
    assert!(!path.exists(), "open created nothing");
    let (vault, key) = VaultStore::create(&path, "pp").unwrap();
    vault.put(&VaultKey::AuditTail, b"persisted").unwrap();
    drop(vault);
    assert!(matches!(
        VaultStore::create(&path, "pp"),
        Err(VaultError::AlreadyExists)
    ));
    let (vault, again) = VaultStore::open(&path, "pp").unwrap();
    assert_eq!(
        &*vault.get(&VaultKey::AuditTail).unwrap().unwrap(),
        b"persisted"
    );
    assert_eq!(key.as_bytes(), again.as_bytes(), "the app-db key is stable");
}

#[test]
fn a_wrong_passphrase_is_reported_as_one() {
    let (_dir, path) = fresh_path();
    drop(VaultStore::create(&path, "right").unwrap());
    assert!(matches!(
        VaultStore::open(&path, "wrong"),
        Err(VaultError::WrongPassphrase)
    ));
    assert!(VaultStore::open(&path, "right").is_ok());
}

#[test]
fn a_missing_header_is_reported() {
    let (_dir, path) = fresh_path();
    drop(VaultStore::create(&path, "pp").unwrap());
    std::fs::remove_file(crate::header::header_path(&path)).unwrap();
    assert!(matches!(
        VaultStore::open(&path, "pp"),
        Err(VaultError::HeaderMissing)
    ));
}

#[test]
fn round_trip_put_get_delete_and_list() {
    let (_dir, path) = fresh_path();
    let (vault, _) = VaultStore::create(&path, "pp").unwrap();
    let k = VaultKey::AuditMacKey;
    vault.put(&k, b"hello world").unwrap();
    assert_eq!(&*vault.get(&k).unwrap().unwrap(), b"hello world");
    let mek = |community: &str| VaultKey::CommunityMek {
        community: community.into(),
    };
    vault.put(&mek("c7"), b"x").unwrap();
    vault.put(&mek("c9"), b"y").unwrap();
    let listed = vault.list("communities").unwrap();
    assert_eq!(listed, vec![mek("c7"), mek("c9")]);
    vault.delete(&k).unwrap();
    assert!(vault.get(&k).unwrap().is_none());
}

/// An entry's ciphertext is bound to its address: moved under another
/// key it does not open.
#[test]
fn an_entry_moved_to_another_key_does_not_open() {
    let (_dir, path) = fresh_path();
    let (vault, _) = VaultStore::create(&path, "pp").unwrap();
    let mek = |community: &str| VaultKey::CommunityMek {
        community: community.into(),
    };
    vault.put(&mek("c1"), b"one").unwrap();
    {
        let conn = vault.conn.lock();
        conn.execute(
            "UPDATE entries SET key = ?1 WHERE key = ?2",
            params![mek("c2").key().as_ref(), mek("c1").key().as_ref()],
        )
        .unwrap();
    }
    assert!(vault.get(&mek("c2")).is_err());
}

#[test]
fn delete_files_removes_the_vault_and_its_header() {
    let (_dir, path) = fresh_path();
    drop(VaultStore::create(&path, "pp").unwrap());
    VaultStore::delete_files(&path).unwrap();
    assert!(!path.exists());
    assert!(!crate::header::header_path(&path).exists());
    VaultStore::delete_files(&path).unwrap();
}
