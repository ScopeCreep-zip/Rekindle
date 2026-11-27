use super::*;
use crate::VaultStore;

fn vault(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("a.vault");
    drop(VaultStore::create(&path, "real").unwrap());
    path
}

#[test]
fn an_armed_duress_passphrase_opens_nothing_and_says_duress() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = vault(&dir);
    arm(&path, "duress").unwrap();
    assert!(matches!(
        VaultStore::open(&path, "duress"),
        Err(VaultError::Duress)
    ));
    assert!(VaultStore::open(&path, "real").is_ok());
    assert!(matches!(
        VaultStore::open(&path, "other"),
        Err(VaultError::WrongPassphrase)
    ));
}

#[test]
fn disarming_forgets_it_and_the_header_stays_the_same_size() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = vault(&dir);
    let before = header::read(&path).unwrap();
    arm(&path, "duress").unwrap();
    let armed = header::read(&path).unwrap();
    assert_eq!(armed.salt, before.salt, "the real salt is kept");
    assert_ne!(armed.duress_salt, before.duress_salt);
    disarm(&path).unwrap();
    assert!(matches!(
        VaultStore::open(&path, "duress"),
        Err(VaultError::WrongPassphrase)
    ));
    let on_disk = std::fs::read(header::header_path(&path)).unwrap();
    assert_eq!(on_disk.len(), header::VAULT_HEADER_LEN);
    assert!(VaultStore::open(&path, "real").is_ok());
}

/// Timing parity (plan E7.1; research cross-check 2026-11-19 §6): the
/// duress passphrase, the real one and a wrong one each run the same
/// two Argon2id derivations, armed or not, so the KDF time (the cost
/// that dominates an open) does not tell them apart.
#[test]
fn every_open_runs_both_derivations_armed_or_not() {
    let stretches = |path: &std::path::Path, pass: &str| {
        header::STRETCHES.with(|n| n.set(0));
        let _ = VaultStore::open(path, pass);
        header::STRETCHES.with(std::cell::Cell::get)
    };
    let dir = tempfile::TempDir::new().unwrap();
    let path = vault(&dir);
    let unarmed = [stretches(&path, "real"), stretches(&path, "other")];
    arm(&path, "duress").unwrap();
    let armed = [
        stretches(&path, "duress"),
        stretches(&path, "real"),
        stretches(&path, "other"),
    ];
    assert_eq!(unarmed, [2, 2]);
    assert_eq!(armed, [2, 2, 2]);
}
