//! In-memory implementations of Signal Protocol storage traits.
//!
//! These are suitable for:
//! - Testing and development
//! - Runtime fallback before Stronghold integration is complete
//!
//! **WARNING**: Data is lost on process exit. For production use,
//! implement the traits using Stronghold + `SQLite` via the Tauri backend.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::signal::store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use crate::CryptoError;

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
        let trusted = self.trusted.lock();
        match trusted.get(address) {
            Some(stored) => Ok(stored == identity_key),
            None => Ok(true), // TOFU: trust on first use
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError> {
        self.trusted
            .lock()
            .insert(address.to_string(), identity_key.to_vec());
        Ok(())
    }
}

/// In-memory prekey store.
///
/// Stores one-time prekeys and signed prekeys in memory. One-time keys
/// are kept in insertion order so listing returns them oldest first.
pub struct MemoryPreKeyStore {
    prekeys: Mutex<Vec<(u32, Vec<u8>)>>,
    signed_prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    pq_last_resort: Mutex<HashMap<u32, Vec<u8>>>,
    pq_one_time: Mutex<Vec<(u32, Vec<u8>)>>,
}

impl MemoryPreKeyStore {
    pub fn new() -> Self {
        Self {
            prekeys: Mutex::new(Vec::new()),
            signed_prekeys: Mutex::new(HashMap::new()),
            pq_last_resort: Mutex::new(HashMap::new()),
            pq_one_time: Mutex::new(Vec::new()),
        }
    }
}

fn ordered_get(entries: &[(u32, Vec<u8>)], id: u32) -> Option<Vec<u8>> {
    entries
        .iter()
        .find(|(entry_id, _)| *entry_id == id)
        .map(|(_, data)| data.clone())
}

/// Replace in place (keeping the slot's age) or append as newest.
fn ordered_put(entries: &mut Vec<(u32, Vec<u8>)>, id: u32, data: &[u8]) {
    if let Some(slot) = entries.iter_mut().find(|(entry_id, _)| *entry_id == id) {
        slot.1 = data.to_vec();
    } else {
        entries.push((id, data.to_vec()));
    }
}

impl Default for MemoryPreKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PreKeyStore for MemoryPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(ordered_get(&self.prekeys.lock(), prekey_id))
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        ordered_put(&mut self.prekeys.lock(), prekey_id, key_data);
        Ok(())
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        self.prekeys.lock().retain(|(id, _)| *id != prekey_id);
        Ok(())
    }

    fn list_prekey_ids(&self) -> Result<Vec<u32>, CryptoError> {
        Ok(self.prekeys.lock().iter().map(|(id, _)| *id).collect())
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.signed_prekeys.lock().get(&signed_prekey_id).cloned())
    }

    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        self.signed_prekeys
            .lock()
            .insert(signed_prekey_id, key_data.to_vec());
        Ok(())
    }

    fn load_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
    ) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(match kind {
            PqKeyKind::LastResort => self.pq_last_resort.lock().get(&prekey_id).cloned(),
            PqKeyKind::OneTime => ordered_get(&self.pq_one_time.lock(), prekey_id),
        })
    }

    fn store_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        match kind {
            PqKeyKind::LastResort => {
                self.pq_last_resort
                    .lock()
                    .insert(prekey_id, key_data.to_vec());
            }
            PqKeyKind::OneTime => ordered_put(&mut self.pq_one_time.lock(), prekey_id, key_data),
        }
        Ok(())
    }

    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<(), CryptoError> {
        if kind == PqKeyKind::OneTime {
            self.pq_one_time.lock().retain(|(id, _)| *id != prekey_id);
        }
        Ok(())
    }

    fn list_pq_one_time_ids(&self) -> Result<Vec<u32>, CryptoError> {
        Ok(self.pq_one_time.lock().iter().map(|(id, _)| *id).collect())
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
        Ok(self.sessions.lock().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        self.sessions
            .lock()
            .insert(address.to_string(), session_data.to_vec());
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        Ok(self.sessions.lock().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        self.sessions.lock().remove(address);
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        Ok(self.sessions.lock().keys().cloned().collect())
    }
}
