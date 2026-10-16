//! Vault-backed Signal Protocol storage: the one Signal store owner from
//! D1 to E2.3 (plan D1), shared by both hosts over
//! [`rekindle_vault::VaultHandle`]. E2.3 replaces it with the
//! `rekindle_db::ratchet` tables.
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

use rekindle_vault::typed::signal::{
    delete_signal_pq_one_time, delete_signal_prekey, delete_signal_session,
    list_signal_pq_one_time_ids, list_signal_prekey_ids, list_signal_sessions,
    load_signal_pq_secret, load_signal_prekey, load_signal_session, load_signal_signed_prekey,
    load_trusted_identity, persist_signal_pq_secret, persist_signal_prekey, persist_signal_session,
    persist_signal_signed_prekey, persist_trusted_identity,
};
use rekindle_vault::{VaultHandle, VaultStore};

/// Vault-backed identity key store with TOFU on save_identity.
pub struct VaultIdentityStore {
    keystore: VaultHandle,
    cache: Arc<MemoryIdentityStore>,
}

impl VaultIdentityStore {
    /// Construct a new store and prime the in-memory TOFU cache from
    /// previously-persisted trusted identities. Identity keypair +
    /// registration ID come from the caller (auth.rs has them at hand).
    pub fn new(
        keystore: VaultHandle,
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

impl IdentityKeyStore for VaultIdentityStore {
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
pub struct VaultPreKeyStore {
    keystore: VaultHandle,
    cache: MemoryPreKeyStore,
}

impl VaultPreKeyStore {
    /// Load every persisted prekey into the cache. Fails if the vault is
    /// locked or any read fails.
    pub fn new(keystore: VaultHandle) -> Result<Self, CryptoError> {
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
        op: impl FnOnce(&VaultStore) -> Result<(), String>,
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
fn prime_prekeys(vault: &VaultStore, cache: &MemoryPreKeyStore) -> Result<(), CryptoError> {
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

impl PreKeyStore for VaultPreKeyStore {
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
pub struct VaultSessionStore {
    keystore: VaultHandle,
    cache: MemorySessionStore,
}

impl VaultSessionStore {
    /// Load every persisted session into the cache. Fails if the vault is
    /// locked or any read fails.
    pub fn new(keystore: VaultHandle) -> Result<Self, CryptoError> {
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

impl SessionStore for VaultSessionStore {
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
/// via `with_session_cache` (see `host::identity::unlock`),
/// using the existing sync `SessionStore` as both source of truth and
/// cache backend through an internal adapter. This `VaultSessionPersistence`
/// remains here as an alternative async-native persistence path —
/// useful for the daemon-track integration where the underlying
/// storage may itself be async (e.g. SQLite via tokio_rusqlite).
pub struct VaultSessionPersistence {
    keystore: VaultHandle,
}

impl VaultSessionPersistence {
    pub fn new(keystore: VaultHandle) -> Self {
        Self { keystore }
    }
}

#[async_trait::async_trait]
impl rekindle_crypto::signal::SessionPersistence for VaultSessionPersistence {
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
mod tests;
