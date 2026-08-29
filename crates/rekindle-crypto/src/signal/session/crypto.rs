//! Encrypt/decrypt message paths: the cache-routed and legacy
//! unsynchronized variants, plus the pure ratchet-mutate steps they
//! share.

use crate::error::CryptoError;
use crate::signal::ratchet::{self, RatchetState};
use crate::signal::session_cache::SessionCache;

use super::SignalSessionManager;

impl SignalSessionManager {
    /// Encrypt a plaintext message for a peer.
    ///
    /// Phase 6 — when a [`SessionCache`] is wired via
    /// [`Self::with_session_cache`], this routes load-mutate-store
    /// through the cache's per-peer `tokio::sync::Mutex`. Concurrent
    /// encrypts to the SAME peer serialize on that mutex (ratchet
    /// counters advance in order); encrypts to DIFFERENT peers run in
    /// parallel. Without a cache, falls back to the legacy
    /// unsynchronized path which races under contention.
    pub async fn encrypt(
        &self,
        peer_address: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        if let Some(cache) = self.cache.as_ref() {
            return self
                .encrypt_with_cache(cache, peer_address, plaintext)
                .await;
        }
        // Legacy path — test fixtures + callers that haven't wired the cache.
        self.encrypt_unsynchronized(peer_address, plaintext)
    }

    async fn encrypt_with_cache(
        &self,
        cache: &SessionCache,
        peer_address: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let arc = cache.get_or_load(peer_address).await?;
        // Per-peer lock — held only across the in-process mutate. Other
        // peers' encrypts proceed concurrently on independent shards.
        let mut guard = arc.lock().await;
        let mut ratchet = RatchetState::deserialize(&guard)?;
        let (output, new_data) = Self::encrypt_mutate(&mut ratchet, plaintext)?;
        // Update cache snapshot AND persist to durable store. Persisting
        // under the per-peer lock guarantees the durable store's view
        // matches the in-memory snapshot once the lock is released.
        guard.clone_from(&new_data);
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(output)
    }

    fn encrypt_unsynchronized(
        &self,
        peer_address: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let session_data = self
            .sessions
            .load_session(peer_address)?
            .ok_or_else(|| CryptoError::SessionError("no session for peer".into()))?;
        let mut ratchet = RatchetState::deserialize(&session_data)?;
        let (output, new_data) = Self::encrypt_mutate(&mut ratchet, plaintext)?;
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(output)
    }

    /// Pure mutate step shared by cache and non-cache paths. Returns
    /// `(wire_output, new_session_bytes)`. Caller is responsible for
    /// persisting `new_session_bytes` (under the per-peer lock when
    /// the cache is in play). Stepping lives in the shared ratchet
    /// core ([`crate::signal::ratchet`]).
    fn encrypt_mutate(
        ratchet: &mut RatchetState,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        let output = ratchet.encrypt_step(plaintext)?;
        Ok((output, ratchet.serialize()))
    }

    /// Decrypt a ciphertext message from a peer.
    ///
    /// Phase 6 — see [`Self::encrypt`]. Same cache-vs-fallback semantics.
    pub async fn decrypt(
        &self,
        peer_address: &str,
        message: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        if message.len() < ratchet::HEADER_LEN {
            return Err(CryptoError::DecryptionError("message too short".into()));
        }
        if let Some(cache) = self.cache.as_ref() {
            return self.decrypt_with_cache(cache, peer_address, message).await;
        }
        self.decrypt_unsynchronized(peer_address, message)
    }

    async fn decrypt_with_cache(
        &self,
        cache: &SessionCache,
        peer_address: &str,
        message: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let arc = cache.get_or_load(peer_address).await?;
        let mut guard = arc.lock().await;
        let mut ratchet = RatchetState::deserialize(&guard)?;
        let (plaintext, new_data) = Self::decrypt_mutate(&mut ratchet, message)?;
        guard.clone_from(&new_data);
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(plaintext)
    }

    fn decrypt_unsynchronized(
        &self,
        peer_address: &str,
        message: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let session_data = self
            .sessions
            .load_session(peer_address)?
            .ok_or_else(|| CryptoError::SessionError("no session for peer".into()))?;
        let mut ratchet = RatchetState::deserialize(&session_data)?;
        let (plaintext, new_data) = Self::decrypt_mutate(&mut ratchet, message)?;
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(plaintext)
    }

    fn decrypt_mutate(
        ratchet: &mut RatchetState,
        message: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        let plaintext = ratchet.decrypt_step(message)?;
        Ok((plaintext, ratchet.serialize()))
    }
}
