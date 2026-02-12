//! In-memory implementations of Signal Protocol storage traits.
//!
//! These are suitable for:
//! - Testing and development
//! - Runtime fallback before Stronghold integration is complete
//!
//! **WARNING**: Data is lost on process exit. For production use,
//! implement the traits using Stronghold + `SQLite` via the Tauri backend.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::CryptoError;
use crate::signal::store::{IdentityKeyStore, PreKeyStore, SessionStore};

/// In-memory identity key store.
///
/// Stores the local identity key pair and trusted remote identities
/// using Trust On First Use (TOFU) policy.
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
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        Ok((self.identity_private.clone(), self.identity_public.clone()))
    }

    fn get_local_registration_id(&self) -> Result<u32, CryptoError> {
        Ok(self.registration_id)
    }

    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool, CryptoError> {
        let trusted = self.trusted.lock().unwrap();
        match trusted.get(address) {
            Some(stored) => Ok(stored == identity_key),
            None => Ok(true), // TOFU: trust on first use
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError> {
        self.trusted
            .lock()
            .unwrap()
            .insert(address.to_string(), identity_key.to_vec());
        Ok(())
    }
}

/// In-memory prekey store.
///
/// Stores one-time prekeys and signed prekeys in memory.
pub struct MemoryPreKeyStore {
    prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    signed_prekeys: Mutex<HashMap<u32, Vec<u8>>>,
}

impl MemoryPreKeyStore {
    pub fn new() -> Self {
        Self {
            prekeys: Mutex::new(HashMap::new()),
            signed_prekeys: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemoryPreKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PreKeyStore for MemoryPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.prekeys.lock().unwrap().get(&prekey_id).cloned())
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        self.prekeys
            .lock()
            .unwrap()
            .insert(prekey_id, key_data.to_vec());
        Ok(())
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        self.prekeys.lock().unwrap().remove(&prekey_id);
        Ok(())
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self
            .signed_prekeys
            .lock()
            .unwrap()
            .get(&signed_prekey_id)
            .cloned())
    }

    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        self.signed_prekeys
            .lock()
            .unwrap()
            .insert(signed_prekey_id, key_data.to_vec());
        Ok(())
    }
}

/// In-memory session store.
///
/// Stores Signal session state keyed by peer public key (hex-encoded).
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
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.sessions.lock().unwrap().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(address.to_string(), session_data.to_vec());
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        Ok(self.sessions.lock().unwrap().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        self.sessions.lock().unwrap().remove(address);
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        Ok(self.sessions.lock().unwrap().keys().cloned().collect())
    }
}
