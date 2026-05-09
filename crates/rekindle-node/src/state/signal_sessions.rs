//! Persistent Signal Protocol session store backed by the OS keyring.
//!
//! Each Signal session (ratchet state) is persisted under label
//! `signal-session-{peer_short}` where peer_short is the first 16 chars
//! of the peer's hex public key. Sessions are loaded into an in-memory
//! cache on daemon unlock and written back on every ratchet step.

use std::collections::HashMap;
use std::sync::Mutex;

use rekindle_transport::crypto::signal_store::SessionStore;
use rekindle_transport::error::Result;

/// Persistent session store: in-memory cache + OS keyring persistence.
///
/// Reads are served from the in-memory cache (O(1) HashMap lookup).
/// Writes update the cache AND persist to the keyring asynchronously.
/// On daemon restart, `load_all` populates the cache from the keyring.
pub struct KeyringSessionStore {
    sessions: Mutex<HashMap<String, Vec<u8>>>,
}

impl Default for KeyringSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringSessionStore {
    /// Create an empty store. Use `load_all` to populate from keyring.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Load all persisted Signal sessions from the keyring into memory.
    /// Called once during daemon unlock.
    pub async fn load_all(peer_keys: &[String]) -> Self {
        let mut sessions = HashMap::new();
        for peer_key in peer_keys {
            let label = Self::label(peer_key);
            if let Ok(Some(bytes)) = crate::state::keystore::load_keypair_bytes(&label).await {
                sessions.insert(peer_key.clone(), bytes);
            }
        }
        Self {
            sessions: Mutex::new(sessions),
        }
    }

    fn label(peer_key: &str) -> String {
        let short = &peer_key[..16.min(peer_key.len())];
        format!("signal-session-{short}")
    }
}

impl SessionStore for KeyringSessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.sessions.lock().unwrap().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<()> {
        self.sessions
            .lock()
            .unwrap()
            .insert(address.to_string(), session_data.to_vec());
        // Persist to keyring asynchronously — the in-memory cache is the
        // authoritative source during runtime. The keyring is for restart recovery.
        let label = Self::label(address);
        let data = session_data.to_vec();
        tokio::task::spawn(async move {
            if let Err(e) = crate::state::keystore::store_keypair_bytes(&label, &data).await {
                tracing::warn!(error = %e, "Signal session keyring persist failed");
            }
        });
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool> {
        Ok(self.sessions.lock().unwrap().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<()> {
        self.sessions.lock().unwrap().remove(address);
        let label = Self::label(address);
        tokio::task::spawn(async move {
            let _ = crate::state::keystore::delete_keypair_bytes(&label).await;
        });
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>> {
        Ok(self.sessions.lock().unwrap().keys().cloned().collect())
    }
}
