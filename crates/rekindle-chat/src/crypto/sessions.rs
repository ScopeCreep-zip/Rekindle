//! Triple Ratchet session cache: DashMap (1024 shards) + LRU (10K cap).
//!
//! Hot sessions live in memory. Cold sessions are persisted to vault.
//! Lock ordering: DashMap shard → drop → Arc::clone → tokio::sync::Mutex.
//! NEVER hold DashMap entry guard across .await.

use std::num::NonZeroUsize;
use std::sync::Arc;

use dashmap::DashMap;
use lru::LruCache;
use tokio::sync::Mutex;
use rekindle_ratchet::session::TripleRatchetSession;
use rekindle_storage::VaultStore;

use crate::ChatError;

pub type SessionId = [u8; 32];

const SHARD_COUNT: usize = 1024;
const HOT_CAP: usize = 10_000;

pub struct SessionCache {
    map: DashMap<SessionId, Arc<Mutex<TripleRatchetSession>>>,
    lru: Mutex<LruCache<SessionId, ()>>,
    vault: Arc<VaultStore>,
}

impl SessionCache {
    pub fn new(vault: Arc<VaultStore>) -> Self {
        Self {
            map: DashMap::with_shard_amount(SHARD_COUNT),
            lru: Mutex::new(LruCache::new(
                NonZeroUsize::new(HOT_CAP).expect("nonzero"),
            )),
            vault,
        }
    }

    /// Acquire session lock. Clone Arc, drop DashMap guard, then await Mutex.
    pub async fn with_session<F, R>(
        &self,
        id: &SessionId,
        f: F,
    ) -> Result<R, ChatError>
    where
        F: FnOnce(&mut TripleRatchetSession) -> Result<R, ChatError>,
    {
        let arc = self
            .map
            .get(id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| ChatError::NoSession {
                peer_key: hex::encode(id),
            })?;
        // DashMap entry guard dropped — shard lock released

        let mut guard = arc.lock().await;
        let result = f(&mut guard)?;

        // Touch LRU (lock ordering: session → LRU)
        self.lru.lock().await.get(id);

        Ok(result)
    }

    /// Insert a new session into the hot cache.
    pub async fn insert(&self, id: SessionId, session: TripleRatchetSession) {
        self.map.insert(id, Arc::new(Mutex::new(session)));
        self.lru.lock().await.put(id, ());
    }

    /// Load from vault if not already in hot cache.
    pub async fn ensure_loaded(&self, peer_key: &str) -> Result<SessionId, ChatError> {
        // Check vault for a session with this peer
        let (id, cbor) = self
            .vault
            .load_session_by_peer(peer_key)?
            .ok_or_else(|| ChatError::NoSession {
                peer_key: peer_key.into(),
            })?;

        if !self.map.contains_key(&id) {
            // Wrap CBOR bytes in Zeroizing so raw ratchet key material
            // (root keys, chain keys, header keys) in the serialized
            // session state is zeroed when this scope exits.
            let cbor = zeroize::Zeroizing::new(cbor);
            let session: TripleRatchetSession =
                cbor4ii::serde::from_slice(&cbor).map_err(|e| {
                    ChatError::Deserialization(format!("session CBOR: {e}"))
                })?;
            self.insert(id, session).await;
        }

        Ok(id)
    }

    /// Persist a session to the vault after a ratchet step.
    pub fn persist(
        &self,
        id: &SessionId,
        peer_key: &str,
        session: &TripleRatchetSession,
    ) -> Result<(), ChatError> {
        let cbor = cbor4ii::serde::to_vec(Vec::new(), session)
            .map_err(|e| ChatError::Serialization(format!("session CBOR: {e}")))?;
        let direction = match session.direction {
            rekindle_ratchet::Direction::Initiator => 0u8,
            rekindle_ratchet::Direction::Responder => 1u8,
        };
        self.vault.store_session(
            id,
            peer_key,
            direction,
            &cbor,
            session.spqr_active,
            0, // trust_level ordinal
        )?;
        Ok(())
    }

    /// Check if a session exists for a peer (hot or cold).
    pub fn has_session_for_peer(&self, peer_key: &str) -> Result<bool, ChatError> {
        Ok(self.vault.has_session_for_peer(peer_key)?)
    }

    /// Clear all sessions from the hot cache. Each TripleRatchetSession is
    /// dropped, firing ZeroizeOnDrop on the DoubleRatchetState and manual
    /// Drop on MlKemBraidState.
    pub async fn clear(&self) {
        self.map.clear();
        self.lru.lock().await.clear();
    }
}
