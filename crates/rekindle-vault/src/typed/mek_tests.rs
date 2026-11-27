use super::*;
use rekindle_types::id::ChannelId;
use tempfile::TempDir;

const CH: KeyScope = KeyScope::Channel(ChannelId([1; 16]));
const OTHER: KeyScope = KeyScope::Channel(ChannelId([2; 16]));

fn keystore(dir: &TempDir) -> VaultStore {
    {
        // Create on first use, reopen after: the tests reopen a vault.
        let path = dir.path().join("rekindle.vault");
        if path.exists() {
            VaultStore::open(&path, "testpass").unwrap().0
        } else {
            VaultStore::create(&path, "testpass").unwrap().0
        }
    }
}

#[test]
fn every_generation_of_every_scope_is_kept() {
    let dir = TempDir::new().unwrap();
    let ks = keystore(&dir);
    let (g1, g2) = (
        MediaEncryptionKey::generate(1),
        MediaEncryptionKey::generate(2),
    );
    for scope in [KeyScope::Community, CH] {
        persist_mek(&ks, "c", scope, &g1).unwrap();
        persist_mek(&ks, "c", scope, &g2).unwrap();
        assert_eq!(
            load_mek_generation(&ks, "c", scope, 1).unwrap().as_bytes(),
            g1.as_bytes()
        );
        assert_eq!(
            load_mek_generation(&ks, "c", scope, 2).unwrap().as_bytes(),
            g2.as_bytes()
        );
        assert_eq!(load_latest_mek(&ks, "c", scope).unwrap().generation(), 2);
        assert!(load_mek_generation(&ks, "c", scope, 3).is_none());
    }
    assert!(load_latest_mek(&ks, "c", OTHER).is_none());
}

/// Storing an older generation late keeps it readable but does not
/// roll the scope's latest key back.
#[test]
fn late_older_generation_does_not_replace_latest() {
    let dir = TempDir::new().unwrap();
    let ks = keystore(&dir);
    persist_mek(
        &ks,
        "c",
        KeyScope::Community,
        &MediaEncryptionKey::generate(5),
    )
    .unwrap();
    persist_mek(
        &ks,
        "c",
        KeyScope::Community,
        &MediaEncryptionKey::generate(4),
    )
    .unwrap();
    assert_eq!(
        load_latest_mek(&ks, "c", KeyScope::Community)
            .unwrap()
            .generation(),
        5
    );
    assert!(load_mek_generation(&ks, "c", KeyScope::Community, 4).is_some());
}

#[test]
fn survives_reopen() {
    let dir = TempDir::new().unwrap();
    let mek = MediaEncryptionKey::generate(42);
    persist_mek(&keystore(&dir), "c", CH, &mek).unwrap();
    let loaded = load_mek_generation(&keystore(&dir), "c", CH, 42).unwrap();
    assert_eq!(loaded.as_bytes(), mek.as_bytes());
}

#[test]
fn delete_erases_every_generation_of_the_scope_only() {
    let dir = TempDir::new().unwrap();
    let ks = keystore(&dir);
    for generation in 1..=3 {
        persist_mek(
            &ks,
            "c",
            KeyScope::Community,
            &MediaEncryptionKey::generate(generation),
        )
        .unwrap();
    }
    persist_mek(&ks, "c", CH, &MediaEncryptionKey::generate(1)).unwrap();
    delete_scope_meks(&ks, "c", KeyScope::Community);
    for generation in 1..=3 {
        assert!(load_mek_generation(&ks, "c", KeyScope::Community, generation).is_none());
    }
    assert!(load_latest_mek(&ks, "c", KeyScope::Community).is_none());
    assert!(load_latest_mek(&ks, "c", CH).is_some());
}
