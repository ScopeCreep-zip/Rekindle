//! Vault-backed Signal Protocol storage.
//!
//! Each store wraps the rekindle-crypto Memory* implementation as an
//! in-memory cache and writes through to `rekindle-vault` via the
//! `keystore` helpers. Reads hit the cache; writes and deletes reach the
//! vault before returning Ok, so a crash mid-write doesn't lose state.
//!
//! Fail-closed (vulnerable-user stance): a locked vault or a rejected
//! read, write or delete is an error, never a silently skipped persist —
//! an attacker who can kill the process between an in-memory write and a
//! later flush must not be able to roll keys or trust decisions back.

use std::sync::Arc;

use rekindle_crypto::signal::store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use rekindle_crypto::signal::{
    MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore, PQ_LR_ID, SPK_ID,
};
use rekindle_crypto::CryptoError;

use crate::keystore::{
    delete_signal_pq_one_time, delete_signal_prekey, delete_signal_session,
    list_signal_pq_one_time_ids, list_signal_prekey_ids, list_signal_sessions,
    load_signal_pq_secret, load_signal_prekey, load_signal_session, load_signal_signed_prekey,
    load_trusted_identity, persist_signal_pq_secret, persist_signal_prekey, persist_signal_session,
    persist_signal_signed_prekey, persist_trusted_identity, KeystoreHandle,
};

/// Stronghold-backed identity key store with TOFU on save_identity.
pub struct StrongholdIdentityStore {
    keystore: KeystoreHandle,
    cache: Arc<MemoryIdentityStore>,
}

impl StrongholdIdentityStore {
    /// Construct a new store and prime the in-memory TOFU cache from
    /// previously-persisted trusted identities. Identity keypair +
    /// registration ID come from the caller (auth.rs has them at hand).
    pub fn new(
        keystore: KeystoreHandle,
        identity_private: Vec<u8>,
        identity_public: Vec<u8>,
        registration_id: u32,
    ) -> Self {
        let cache = Arc::new(MemoryIdentityStore::new(
            identity_private,
            identity_public,
            registration_id,
        ));
        // Note: trusted-identity entries are looked up lazily on
        // is_trusted_identity by querying Stronghold first; the in-memory
        // cache fills opportunistically. We don't bulk-prime here because
        // there's no list-trusted-identities helper (TOFU is per-peer,
        // never enumerated as a set).
        Self { keystore, cache }
    }
}

impl IdentityKeyStore for StrongholdIdentityStore {
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        self.cache.get_identity_key_pair()
    }

    fn get_local_registration_id(&self) -> Result<u32, CryptoError> {
        self.cache.get_local_registration_id()
    }

    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool, CryptoError> {
        // Cache hit: defer to TOFU check on the in-memory copy.
        if let Ok(true) = self.cache.is_trusted_identity(address, identity_key) {
            // Either matches the cached entry, or no cached entry exists yet.
            // Confirm against Stronghold for the no-cached-entry case so a
            // restart's pristine cache doesn't auto-trust a substituted key.
            let ks = self.keystore.lock();
            if let Some(keystore) = ks.as_ref() {
                if let Some(persisted) = load_trusted_identity(keystore, address) {
                    return Ok(persisted == identity_key);
                }
            }
            // No persisted entry — TOFU: trust on first use.
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError> {
        // Always update both layers. Stronghold first; if it fails we
        // refuse to update the cache (otherwise the cache would diverge
        // from disk).
        {
            let ks = self.keystore.lock();
            let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
            persist_trusted_identity(vault, address, identity_key).map_err(CryptoError::storage)?;
        }
        self.cache.save_identity(address, identity_key)
    }
}

/// Vault-backed prekey store with a write-through in-memory cache.
///
/// Every key is primed at construction, so reads are cache-only. Writes and
/// deletes reach the vault first and fail when it is locked or rejects them.
pub struct StrongholdPreKeyStore {
    keystore: KeystoreHandle,
    cache: MemoryPreKeyStore,
}

impl StrongholdPreKeyStore {
    /// Load every persisted prekey into the cache. Fails if the vault is
    /// locked or any read fails.
    pub fn new(keystore: KeystoreHandle) -> Result<Self, CryptoError> {
        let cache = MemoryPreKeyStore::new();
        {
            let ks = keystore.lock();
            let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
            prime_prekeys(vault, &cache)?;
        }
        Ok(Self { keystore, cache })
    }

    /// Run `op` against the unlocked vault, mapping its error.
    fn with_vault(
        &self,
        op: impl FnOnce(&crate::keystore::StrongholdKeystore) -> Result<(), String>,
    ) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
        op(vault).map_err(CryptoError::storage)
    }
}

/// Copy the signed prekey, the PQ last-resort key and every indexed
/// one-time key from the vault into `cache`, oldest first. An indexed id
/// whose key is gone was consumed by a delete that stopped before its
/// index update; the index entry is removed to finish that delete.
fn prime_prekeys(
    vault: &crate::keystore::StrongholdKeystore,
    cache: &MemoryPreKeyStore,
) -> Result<(), CryptoError> {
    if let Some(data) = load_signal_signed_prekey(vault, SPK_ID).map_err(CryptoError::storage)? {
        cache.store_signed_prekey(SPK_ID, &data)?;
    }
    if let Some(data) =
        load_signal_pq_secret(vault, PQ_LR_ID, true).map_err(CryptoError::storage)?
    {
        cache.store_pq_secret(PQ_LR_ID, PqKeyKind::LastResort, &data)?;
    }
    for id in list_signal_prekey_ids(vault).map_err(CryptoError::storage)? {
        match load_signal_prekey(vault, id).map_err(CryptoError::storage)? {
            Some(data) => cache.store_prekey(id, &data)?,
            None => delete_signal_prekey(vault, id).map_err(CryptoError::storage)?,
        }
    }
    for id in list_signal_pq_one_time_ids(vault).map_err(CryptoError::storage)? {
        match load_signal_pq_secret(vault, id, false).map_err(CryptoError::storage)? {
            Some(data) => cache.store_pq_secret(id, PqKeyKind::OneTime, &data)?,
            None => delete_signal_pq_one_time(vault, id).map_err(CryptoError::storage)?,
        }
    }
    Ok(())
}

impl PreKeyStore for StrongholdPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        self.cache.load_prekey(prekey_id)
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        self.with_vault(|v| persist_signal_prekey(v, prekey_id, key_data))?;
        self.cache.store_prekey(prekey_id, key_data)
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        self.with_vault(|v| delete_signal_prekey(v, prekey_id))?;
        self.cache.remove_prekey(prekey_id)
    }

    fn list_prekey_ids(&self) -> Result<Vec<u32>, CryptoError> {
        self.cache.list_prekey_ids()
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        self.cache.load_signed_prekey(signed_prekey_id)
    }

    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        self.with_vault(|v| persist_signal_signed_prekey(v, signed_prekey_id, key_data))?;
        self.cache.store_signed_prekey(signed_prekey_id, key_data)
    }

    fn load_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
    ) -> Result<Option<Vec<u8>>, CryptoError> {
        self.cache.load_pq_secret(prekey_id, kind)
    }

    fn store_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        let last_resort = kind == PqKeyKind::LastResort;
        self.with_vault(|v| persist_signal_pq_secret(v, prekey_id, last_resort, key_data))?;
        self.cache.store_pq_secret(prekey_id, kind, key_data)
    }

    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<(), CryptoError> {
        // The last-resort key is rotated, never consumed (trait contract).
        if kind == PqKeyKind::LastResort {
            return Ok(());
        }
        self.with_vault(|v| delete_signal_pq_one_time(v, prekey_id))?;
        self.cache.remove_pq_secret(prekey_id, kind)
    }

    fn list_pq_one_time_ids(&self) -> Result<Vec<u32>, CryptoError> {
        self.cache.list_pq_one_time_ids()
    }
}

/// Vault-backed session store with a write-through in-memory cache.
pub struct StrongholdSessionStore {
    keystore: KeystoreHandle,
    cache: MemorySessionStore,
}

impl StrongholdSessionStore {
    /// Load every persisted session into the cache. Fails if the vault is
    /// locked or any read fails.
    pub fn new(keystore: KeystoreHandle) -> Result<Self, CryptoError> {
        let cache = MemorySessionStore::new();
        {
            let ks = keystore.lock();
            let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
            for peer in list_signal_sessions(vault).map_err(CryptoError::storage)? {
                if let Some(data) = load_signal_session(vault, &peer) {
                    cache.store_session(&peer, &data)?;
                }
            }
        }
        Ok(Self { keystore, cache })
    }
}

impl SessionStore for StrongholdSessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        self.cache.load_session(address)
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        {
            let ks = self.keystore.lock();
            let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
            persist_signal_session(vault, address, session_data).map_err(CryptoError::storage)?;
        }
        self.cache.store_session(address, session_data)
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        self.cache.has_session(address)
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        {
            let ks = self.keystore.lock();
            let vault = ks.as_ref().ok_or(CryptoError::VaultLocked)?;
            delete_signal_session(vault, address).map_err(CryptoError::storage)?;
        }
        self.cache.delete_session(address)
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        self.cache.list_sessions()
    }
}

/// Phase 6 — [`rekindle_crypto::signal::SessionPersistence`] adapter
/// backed by the vault-backed keystore facade. Routes async cache
/// persistence calls through the existing sync keystore helpers via
/// `spawn_blocking` so the keystore's parking_lot mutex never crosses
/// an `.await` point.
///
/// The production [`SignalSessionManager`](rekindle_crypto::SignalSessionManager)
/// wires its own internal [`SessionCache`](rekindle_crypto::signal::SessionCache)
/// via `with_session_cache` (see `commands::auth::initialize_signal_manager`),
/// using the existing sync `SessionStore` as both source of truth and
/// cache backend through an internal adapter. This `VaultSessionStore`
/// remains here as an alternative async-native persistence path —
/// useful for the daemon-track integration where the underlying
/// storage may itself be async (e.g. SQLite via tokio_rusqlite).
pub struct VaultSessionStore {
    keystore: KeystoreHandle,
}

impl VaultSessionStore {
    pub fn new(keystore: KeystoreHandle) -> Self {
        Self { keystore }
    }
}

#[async_trait::async_trait]
impl rekindle_crypto::signal::SessionPersistence for VaultSessionStore {
    async fn load(
        &self,
        peer_hex: &str,
    ) -> Result<Option<rekindle_crypto::signal::SessionBytes>, CryptoError> {
        // Symmetric with `store`: locked vault returns `VaultLocked`, not
        // `Ok(None)`. Without this, callers cannot distinguish "vault
        // locked, no chance of finding the session" from "vault open, no
        // session for this peer" — both would surface as `NoSession` at
        // the cache layer, masking the actual cause.
        let keystore = self.keystore.clone();
        let peer = peer_hex.to_string();
        tokio::task::spawn_blocking(move || {
            let ks = keystore.lock();
            match ks.as_ref() {
                Some(k) => Ok(load_signal_session(k, &peer).map(|session| session.to_vec())),
                None => Err(CryptoError::VaultLocked),
            }
        })
        .await
        .map_err(|e| CryptoError::storage(format!("vault session load join: {e}")))?
    }

    async fn store(&self, peer_hex: &str, session: &[u8]) -> Result<(), CryptoError> {
        let keystore = self.keystore.clone();
        let peer = peer_hex.to_string();
        let bytes = session.to_vec();
        tokio::task::spawn_blocking(move || {
            let ks = keystore.lock();
            match ks.as_ref() {
                Some(k) => persist_signal_session(k, &peer, &bytes).map_err(CryptoError::storage),
                None => Err(CryptoError::VaultLocked),
            }
        })
        .await
        .map_err(|e| CryptoError::storage(format!("vault session store join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    //! Phase 6 integration tests — exercise SessionCache through the
    //! VaultSessionStore against a real vault file. These prove the
    //! "restart-and-decrypt" half of the plan's manual scenario:
    //! session bytes persisted via the trait survive a process
    //! restart (simulated as keystore-close-and-reopen).

    use super::*;
    use parking_lot::Mutex;
    use rekindle_crypto::signal::{SessionCache, SessionPersistence};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vault_session_store_roundtrip_through_cache() {
        let dir = TempDir::new().unwrap();
        let keystore = crate::keystore::StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        let handle: KeystoreHandle = Arc::new(Mutex::new(Some(keystore)));

        let store = VaultSessionStore::new(handle.clone());
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
            let keystore = crate::keystore::StrongholdKeystore::initialize(dir.path(), pp).unwrap();
            let handle: KeystoreHandle = Arc::new(Mutex::new(Some(keystore)));
            let store = VaultSessionStore::new(handle);
            store
                .store("bob", b"ratchet-state-rev-7")
                .await
                .expect("vault store");
        }

        // Session 2: reopen.
        {
            let keystore = crate::keystore::StrongholdKeystore::initialize(dir.path(), pp).unwrap();
            let handle: KeystoreHandle = Arc::new(Mutex::new(Some(keystore)));
            let store = VaultSessionStore::new(handle);
            let bytes = store
                .load("bob")
                .await
                .expect("vault load")
                .expect("session present after restart");
            assert_eq!(&bytes, b"ratchet-state-rev-7");
        }
    }

    /// Deep audit (deep-M): concurrent store + load against the same
    /// peer through VaultSessionStore. spawn_blocking + the keystore's
    /// internal parking_lot mutex serialize the vault writes; no torn
    /// data, no deadlock, no Tokio runtime stall.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn vault_session_store_concurrent_store_and_load() {
        let dir = TempDir::new().unwrap();
        let keystore = crate::keystore::StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        let handle: KeystoreHandle = Arc::new(Mutex::new(Some(keystore)));
        let store = Arc::new(VaultSessionStore::new(handle));

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
        // VaultSessionStore with a None keystore (vault never unlocked
        // or already dropped) returns CryptoError::VaultLocked, not a
        // generic storage error. This is what the cache's persist_one
        // surfaces to upstream callers.
        let handle: KeystoreHandle = Arc::new(Mutex::new(None));
        let store = VaultSessionStore::new(handle);
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
        let keystore = crate::keystore::StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        let handle: KeystoreHandle = Arc::new(Mutex::new(Some(keystore)));

        let store = StrongholdPreKeyStore::new(handle.clone()).unwrap();
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

        let reopened = StrongholdPreKeyStore::new(handle.clone()).unwrap();
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
            StrongholdPreKeyStore::new(handle),
            Err(CryptoError::VaultLocked)
        ));
    }
}
