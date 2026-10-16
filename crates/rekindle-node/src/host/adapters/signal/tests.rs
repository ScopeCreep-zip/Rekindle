//! Phase 6 integration tests — exercise SessionCache through the
//! VaultSessionPersistence against a real vault file. These prove the
//! "restart-and-decrypt" half of the plan's manual scenario:
//! session bytes persisted via the trait survive a process
//! restart (simulated as keystore-close-and-reopen).

use super::*;

/// The test vault in `dir`: created on first use, reopened after (the
/// tests reopen it to simulate a restart).
fn test_vault(dir: &std::path::Path, pass: &str) -> Result<VaultStore, rekindle_vault::VaultError> {
    let path = dir.join("t.vault");
    if path.exists() {
        VaultStore::open(&path, pass).map(|(v, _)| v)
    } else {
        VaultStore::create(&path, pass).map(|(v, _)| v)
    }
}
use parking_lot::Mutex;
use rekindle_crypto::signal::{SessionCache, SessionPersistence};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vault_session_store_roundtrip_through_cache() {
    let dir = TempDir::new().unwrap();
    let keystore = test_vault(dir.path(), "pp").unwrap();
    let handle: VaultHandle = Arc::new(Mutex::new(Some(keystore)));

    let store = VaultSessionPersistence::new(handle.clone());
    // Direct store via the persistence trait.
    store
        .store("alice", b"session-bytes-v1")
        .await
        .expect("vault store");
    // Load through a fresh SessionCache (no in-memory state) — must
    // reach the vault to fetch the bytes.
    let cache = SessionCache::new(Arc::new(store), 16);
    let arc = cache.get_or_load("alice").await.expect("cache load");
    let bytes = arc.lock().await;
    assert_eq!(&*bytes, b"session-bytes-v1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_persists_session_bytes() {
    // Plan's manual scenario half: persist session, "restart"
    // (drop keystore + reopen with same passphrase + new cache),
    // verify the session is still loadable.
    let dir = TempDir::new().unwrap();
    let pp = "restart-test";

    // Session 1: write.
    {
        let keystore = test_vault(dir.path(), pp).unwrap();
        let handle: VaultHandle = Arc::new(Mutex::new(Some(keystore)));
        let store = VaultSessionPersistence::new(handle);
        store
            .store("bob", b"ratchet-state-rev-7")
            .await
            .expect("vault store");
    }

    // Session 2: reopen.
    {
        let keystore = test_vault(dir.path(), pp).unwrap();
        let handle: VaultHandle = Arc::new(Mutex::new(Some(keystore)));
        let store = VaultSessionPersistence::new(handle);
        let bytes = store
            .load("bob")
            .await
            .expect("vault load")
            .expect("session present after restart");
        assert_eq!(&bytes, b"ratchet-state-rev-7");
    }
}

/// Deep audit (deep-M): concurrent store + load against the same
/// peer through VaultSessionPersistence. spawn_blocking + the keystore's
/// internal parking_lot mutex serialize the vault writes; no torn
/// data, no deadlock, no Tokio runtime stall.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vault_session_store_concurrent_store_and_load() {
    let dir = TempDir::new().unwrap();
    let keystore = test_vault(dir.path(), "pp").unwrap();
    let handle: VaultHandle = Arc::new(Mutex::new(Some(keystore)));
    let store = Arc::new(VaultSessionPersistence::new(handle));

    // Seed once.
    store.store("alice", b"v0").await.unwrap();

    // Race 20 writes against 20 reads, all targeting "alice".
    let mut tasks = Vec::new();
    for i in 0..20u8 {
        let s = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            s.store("alice", &[i; 8]).await.unwrap();
        }));
        let s = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            // Load must always succeed (some value present); it's
            // racing with concurrent writes, so the read may see
            // any historical write.
            let _ = s.load("alice").await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    // Final load — vault must have some valid 8-byte payload.
    let final_bytes = store.load("alice").await.unwrap().unwrap();
    assert_eq!(
        final_bytes.len(),
        8,
        "vault state is exactly one write's bytes"
    );
}

#[tokio::test]
async fn vault_locked_returns_typed_error() {
    // VaultSessionPersistence with a None keystore (vault never unlocked
    // or already dropped) returns CryptoError::VaultLocked, not a
    // generic storage error. This is what the cache's persist_one
    // surfaces to upstream callers.
    let handle: VaultHandle = Arc::new(Mutex::new(None));
    let store = VaultSessionPersistence::new(handle);
    let store_err = store.store("alice", b"data").await.unwrap_err();
    assert!(
        matches!(store_err, CryptoError::VaultLocked),
        "expected VaultLocked from store, got {store_err:?}",
    );
    // Symmetric on load — must not silently return Ok(None), which
    // would mask "vault locked" as "no session for peer" at the
    // cache layer.
    let load_err = store.load("alice").await.unwrap_err();
    assert!(
        matches!(load_err, CryptoError::VaultLocked),
        "expected VaultLocked from load, got {load_err:?}",
    );
}

/// One-time keys survive a reopen in insertion order, consumed keys
/// stay gone, and a locked vault is an error rather than a skipped
/// write.
#[test]
fn prekey_store_persists_order_and_fails_closed() {
    let dir = TempDir::new().unwrap();
    let keystore = test_vault(dir.path(), "pp").unwrap();
    let handle: VaultHandle = Arc::new(Mutex::new(Some(keystore)));

    let store = VaultPreKeyStore::new(handle.clone()).unwrap();
    for (id, byte) in [(5, 5u8), (3, 3), (9, 9)] {
        store.store_prekey(id, &[byte; 32]).unwrap();
    }
    store.remove_prekey(3).unwrap();
    for (id, byte) in [(8, 8u8), (7, 7)] {
        store
            .store_pq_secret(id, PqKeyKind::OneTime, &[byte; 4])
            .unwrap();
    }
    store
        .store_pq_secret(PQ_LR_ID, PqKeyKind::LastResort, b"lr")
        .unwrap();
    store.store_signed_prekey(SPK_ID, &[1; 32]).unwrap();
    drop(store);

    let reopened = VaultPreKeyStore::new(handle.clone()).unwrap();
    assert_eq!(reopened.list_prekey_ids().unwrap(), vec![5, 9]);
    assert_eq!(reopened.list_pq_one_time_ids().unwrap(), vec![8, 7]);
    assert_eq!(reopened.load_prekey(9).unwrap(), Some(vec![9; 32]));
    assert_eq!(reopened.load_prekey(3).unwrap(), None);
    assert_eq!(
        reopened
            .load_pq_secret(PQ_LR_ID, PqKeyKind::LastResort)
            .unwrap(),
        Some(b"lr".to_vec())
    );
    assert_eq!(
        reopened.load_signed_prekey(SPK_ID).unwrap(),
        Some(vec![1; 32])
    );

    handle.lock().take();
    assert!(matches!(
        reopened.store_prekey(1, &[0; 32]),
        Err(CryptoError::VaultLocked)
    ));
    assert!(matches!(
        reopened.remove_prekey(5),
        Err(CryptoError::VaultLocked)
    ));
    assert!(matches!(
        VaultPreKeyStore::new(handle),
        Err(CryptoError::VaultLocked)
    ));
}
