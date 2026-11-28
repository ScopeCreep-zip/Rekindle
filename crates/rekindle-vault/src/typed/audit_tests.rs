use super::*;
use tempfile::TempDir;

fn vault(dir: &TempDir) -> VaultStore {
    {
        // Create on first use, reopen after: the tests reopen a vault.
        let path = dir.path().join("rekindle.vault");
        if path.exists() {
            VaultStore::open(&path, "pp").unwrap().0
        } else {
            VaultStore::create(&path, "pp").unwrap().0
        }
    }
}

#[test]
fn audit_mac_key_is_stable_across_calls_and_reopen() {
    // chain.verify() needs key stability across sessions.
    let dir = TempDir::new().unwrap();
    let k1 = load_or_create_audit_mac_key(&vault(&dir)).unwrap();
    let k2 = load_or_create_audit_mac_key(&vault(&dir)).unwrap();
    assert_eq!(k1, k2);
}

#[test]
fn audit_mac_key_regenerates_on_corrupt_length() {
    // A wrong-length key is a corruption signal: regenerate. Existing
    // chain entries become unverifiable; that mismatch IS the tamper
    // signal.
    let dir = TempDir::new().unwrap();
    let v = vault(&dir);
    v.put(&VaultKey::AuditMacKey, b"too-short").unwrap();
    let regenerated = load_or_create_audit_mac_key(&v).unwrap();
    assert_eq!(load_or_create_audit_mac_key(&v).unwrap(), regenerated);
}

#[test]
fn audit_tail_anchor_roundtrips_and_bad_or_missing_is_none() {
    let dir = TempDir::new().unwrap();
    let v = vault(&dir);
    assert!(load_audit_tail(&v).is_none());
    persist_audit_tail(&v, 42, &[9; 32]).unwrap();
    assert_eq!(load_audit_tail(&v), Some((42, [9; 32])));
    // A corrupt entry must not panic: treated as no anchor.
    v.put(&VaultKey::AuditTail, b"only-twelve-").unwrap();
    assert!(load_audit_tail(&v).is_none());
}
