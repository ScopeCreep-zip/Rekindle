//! Storage traits and in-memory implementations for Signal Protocol state.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::error::Result;

// ── Storage traits ──────────────────────────────────────────────────────

/// Storage for Signal Protocol identity keys.
pub trait IdentityKeyStore: Send + Sync {
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>)>;
    fn get_local_registration_id(&self) -> Result<u32>;
    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool>;
    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<()>;
}

/// Classifier for PQXDH ML-KEM prekeys. Parallel to
/// `rekindle_crypto::signal::store::PqKeyKind` — defined here to keep
/// rekindle-transport's Signal Protocol implementation self-contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PqKeyKind {
    LastResort,
    OneTime,
}

/// Storage for Signal Protocol prekeys.
pub trait PreKeyStore: Send + Sync {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>>;
    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<()>;
    fn remove_prekey(&self, prekey_id: u32) -> Result<()>;
    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>>;
    fn store_signed_prekey(&self, signed_prekey_id: u32, key_data: &[u8]) -> Result<()>;
    /// Phase 3b — load an ML-KEM-768 secret by `(id, kind)`.
    fn load_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<Option<Vec<u8>>>;
    /// Phase 3b — store an ML-KEM-768 secret (2400-byte FIPS-203 blob).
    fn store_pq_secret(&self, prekey_id: u32, kind: PqKeyKind, key_data: &[u8]) -> Result<()>;
    /// Phase 3b — remove a consumed PQ one-time prekey. No-op for LastResort.
    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<()>;
}

/// Storage for Signal Protocol sessions.
pub trait SessionStore: Send + Sync {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>>;
    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<()>;
    fn has_session(&self, address: &str) -> Result<bool>;
    fn delete_session(&self, address: &str) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<String>>;
}

// ── In-memory implementations ───────────────────────────────────────────

/// In-memory identity key store with TOFU policy.
pub struct MemoryIdentityStore {
    identity_private: Vec<u8>,
    identity_public: Vec<u8>,
    registration_id: u32,
    trusted: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryIdentityStore {
    pub fn new(identity_private: Vec<u8>, identity_public: Vec<u8>, registration_id: u32) -> Self {
        Self {
            identity_private,
            identity_public,
            registration_id,
            trusted: Mutex::new(HashMap::new()),
        }
    }
}

impl IdentityKeyStore for MemoryIdentityStore {
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((self.identity_private.clone(), self.identity_public.clone()))
    }

    fn get_local_registration_id(&self) -> Result<u32> {
        Ok(self.registration_id)
    }

    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool> {
        let trusted = self.trusted.lock();
        match trusted.get(address) {
            Some(stored) => Ok(stored == identity_key),
            None => Ok(true), // TOFU
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<()> {
        self.trusted
            .lock()
            .insert(address.to_string(), identity_key.to_vec());
        Ok(())
    }
}

/// In-memory prekey store.
pub struct MemoryPreKeyStore {
    prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    signed_prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    pq_secrets: Mutex<HashMap<(u32, PqKeyKind), Vec<u8>>>,
}

impl MemoryPreKeyStore {
    pub fn new() -> Self {
        Self {
            prekeys: Mutex::new(HashMap::new()),
            signed_prekeys: Mutex::new(HashMap::new()),
            pq_secrets: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemoryPreKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PreKeyStore for MemoryPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self.prekeys.lock().get(&prekey_id).cloned())
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<()> {
        self.prekeys
            .lock()
            .insert(prekey_id, key_data.to_vec());
        Ok(())
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<()> {
        self.prekeys.lock().remove(&prekey_id);
        Ok(())
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .signed_prekeys
            .lock()
            .get(&signed_prekey_id)
            .cloned())
    }

    fn store_signed_prekey(&self, signed_prekey_id: u32, key_data: &[u8]) -> Result<()> {
        self.signed_prekeys
            .lock()
            .insert(signed_prekey_id, key_data.to_vec());
        Ok(())
    }

    fn load_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<Option<Vec<u8>>> {
        Ok(self.pq_secrets.lock().get(&(prekey_id, kind)).cloned())
    }

    fn store_pq_secret(&self, prekey_id: u32, kind: PqKeyKind, key_data: &[u8]) -> Result<()> {
        self.pq_secrets
            .lock()
            .insert((prekey_id, kind), key_data.to_vec());
        Ok(())
    }

    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<()> {
        if kind == PqKeyKind::OneTime {
            self.pq_secrets.lock().remove(&(prekey_id, kind));
        }
        Ok(())
    }
}

/// In-memory session store.
pub struct MemorySessionStore {
    sessions: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore for MemorySessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.sessions.lock().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<()> {
        self.sessions
            .lock()
            .insert(address.to_string(), session_data.to_vec());
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool> {
        Ok(self.sessions.lock().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<()> {
        self.sessions.lock().remove(address);
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>> {
        Ok(self.sessions.lock().keys().cloned().collect())
    }
}
