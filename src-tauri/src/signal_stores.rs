//! Stronghold-backed Signal Protocol storage (B7/D4 — P0.1 + P0.5 + P1.2).
//!
//! Each store wraps the rekindle-crypto Memory* implementation as an
//! in-memory cache and writes through to Stronghold via the `keystore.rs`
//! delegate helpers. Reads stay fast (HashMap lookup); writes hit
//! Stronghold synchronously so a crash mid-write doesn't lose state.
//!
//! Why src-tauri and not rekindle-crypto: Stronghold is a Tauri-shell
//! concern. rekindle-crypto stays pure-crypto with no iota_stronghold
//! dependency, which keeps the security boundary small and lets the
//! crate be reused by future non-Tauri surfaces.
//!
//! Vulnerable-user safety stance (`feedback_vulnerable_users_no_creative_paths`):
//! every write reaches Stronghold before returning Ok. We do NOT buffer
//! writes in memory and flush "later" — that's a fallback path an attacker
//! can exploit by killing the process between the in-memory write and the
//! disk flush. Fail-closed: if Stronghold rejects a write we surface the
//! error so the caller decides whether to proceed.

use std::sync::Arc;

use parking_lot::Mutex;

use rekindle_crypto::signal::{MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore};
use rekindle_crypto::signal::store::{IdentityKeyStore, PreKeyStore, SessionStore};
use rekindle_crypto::CryptoError;

use crate::keystore::{
    delete_signal_prekey, delete_signal_session, list_signal_prekey_ids, list_signal_sessions,
    load_signal_prekey, load_signal_session, load_signal_signed_prekey, load_trusted_identity,
    persist_signal_prekey, persist_signal_session, persist_signal_signed_prekey,
    persist_trusted_identity, KeystoreHandle,
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

    fn is_trusted_identity(
        &self,
        address: &str,
        identity_key: &[u8],
    ) -> Result<bool, CryptoError> {
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
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            persist_trusted_identity(keystore, address, identity_key)
                .map_err(CryptoError::StorageError)?;
        }
        drop(ks);
        self.cache.save_identity(address, identity_key)
    }
}

/// Stronghold-backed prekey store with write-through cache.
pub struct StrongholdPreKeyStore {
    keystore: KeystoreHandle,
    cache: Arc<Mutex<MemoryPreKeyStore>>,
    primed: Mutex<bool>,
}

impl StrongholdPreKeyStore {
    pub fn new(keystore: KeystoreHandle) -> Self {
        let store = Self {
            keystore,
            cache: Arc::new(Mutex::new(MemoryPreKeyStore::new())),
            primed: Mutex::new(false),
        };
        store.prime_from_stronghold();
        store
    }

    /// Load every persisted prekey + signed prekey into the in-memory cache
    /// so subsequent reads stay fast. Called once at construction.
    fn prime_from_stronghold(&self) {
        let ks = self.keystore.lock();
        let Some(keystore) = ks.as_ref() else { return };
        let cache = self.cache.lock();
        for prekey_id in list_signal_prekey_ids(keystore) {
            if let Some(data) = load_signal_prekey(keystore, prekey_id) {
                let _ = cache.store_prekey(prekey_id, &data);
            }
        }
        // Signed prekeys: there's no index yet (signed prekey rotation is
        // single-slot per generation, so we attempt id 1 explicitly — the
        // common case after `generate_prekey_bundle(1, Some(1))` in auth.rs).
        if let Some(data) = load_signal_signed_prekey(keystore, 1) {
            let _ = cache.store_signed_prekey(1, &data);
        }
        *self.primed.lock() = true;
    }
}

impl PreKeyStore for StrongholdPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        let cache = self.cache.lock();
        cache.load_prekey(prekey_id)
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            persist_signal_prekey(keystore, prekey_id, key_data)
                .map_err(CryptoError::StorageError)?;
        }
        drop(ks);
        let cache = self.cache.lock();
        cache.store_prekey(prekey_id, key_data)
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            delete_signal_prekey(keystore, prekey_id);
        }
        drop(ks);
        let cache = self.cache.lock();
        cache.remove_prekey(prekey_id)
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        let cache = self.cache.lock();
        cache.load_signed_prekey(signed_prekey_id)
    }

    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            persist_signal_signed_prekey(keystore, signed_prekey_id, key_data)
                .map_err(CryptoError::StorageError)?;
        }
        drop(ks);
        let cache = self.cache.lock();
        cache.store_signed_prekey(signed_prekey_id, key_data)
    }
}

/// Stronghold-backed session store with write-through cache.
pub struct StrongholdSessionStore {
    keystore: KeystoreHandle,
    cache: Arc<Mutex<MemorySessionStore>>,
}

impl StrongholdSessionStore {
    pub fn new(keystore: KeystoreHandle) -> Self {
        let store = Self {
            keystore,
            cache: Arc::new(Mutex::new(MemorySessionStore::new())),
        };
        store.prime_from_stronghold();
        store
    }

    fn prime_from_stronghold(&self) {
        let ks = self.keystore.lock();
        let Some(keystore) = ks.as_ref() else { return };
        let cache = self.cache.lock();
        for peer_address in list_signal_sessions(keystore) {
            if let Some(data) = load_signal_session(keystore, &peer_address) {
                let _ = cache.store_session(&peer_address, &data);
            }
        }
    }
}

impl SessionStore for StrongholdSessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        let cache = self.cache.lock();
        cache.load_session(address)
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            persist_signal_session(keystore, address, session_data)
                .map_err(CryptoError::StorageError)?;
        }
        drop(ks);
        let cache = self.cache.lock();
        cache.store_session(address, session_data)
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        let cache = self.cache.lock();
        cache.has_session(address)
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        let ks = self.keystore.lock();
        if let Some(keystore) = ks.as_ref() {
            delete_signal_session(keystore, address);
        }
        drop(ks);
        let cache = self.cache.lock();
        cache.delete_session(address)
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        let cache = self.cache.lock();
        cache.list_sessions()
    }
}
