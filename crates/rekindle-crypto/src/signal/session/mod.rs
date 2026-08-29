use std::sync::Arc;

use crate::error::CryptoError;
use crate::signal::session_cache::{SessionCache, SessionPersistence};
use crate::signal::store::{IdentityKeyStore, PreKeyStore, SessionStore};

mod bundles;
mod crypto;
mod establish;
mod queries;

/// Fixed identifier for the per-identity ML-KEM-768 last-resort key.
/// Singleton — one per identity at a time; rotates rarely.
pub const PQ_LR_ID: u32 = 0;

/// Metadata produced by initiator-side session establishment.
///
/// The ephemeral key, prekey IDs, and PQXDH ML-KEM ciphertext must be
/// sent to the peer so they can call `respond_to_session()` with
/// matching parameters. Phase 3b of the decomposed-harvest plan added
/// the PQXDH-specific fields (`ml_kem_ciphertext`, `used_ot_pqpk_id`).
pub struct SessionInitInfo {
    /// The initiator's X25519 ephemeral public key.
    pub ephemeral_public_key: Vec<u8>,
    /// Which of the responder's signed prekeys was used.
    pub signed_prekey_id: u32,
    /// Which of the responder's one-time prekeys was consumed (if any).
    pub one_time_prekey_id: Option<u32>,
    /// PQXDH ML-KEM-768 ciphertext (1088 bytes) — the encapsulation
    /// targeted at the responder's chosen PQ key.
    pub ml_kem_ciphertext: Vec<u8>,
    /// Which of the responder's one-time PQ prekeys was consumed
    /// (`None` means the last-resort key at `PQ_LR_ID` was used).
    pub used_ot_pqpk_id: Option<u32>,
}

/// Manages Signal Protocol sessions for 1:1 encrypted messaging.
///
/// Uses PQXDH for session establishment and the shared Double Ratchet
/// core ([`crate::signal::ratchet`]) for forward-secret message
/// encryption — the same stepping and wire format as the daemon track.
pub struct SignalSessionManager {
    identity: Box<dyn IdentityKeyStore>,
    prekeys: Box<dyn PreKeyStore>,
    /// `Arc` (not `Box`) so the underlying store can be shared with the
    /// optional [`SessionCache`] adapter without breaking ownership.
    sessions: Arc<dyn SessionStore>,
    /// Phase 6 — per-peer atomicity cache. When `Some`, [`Self::encrypt`]
    /// and [`Self::decrypt`] route load-mutate-store through the cache's
    /// per-peer `tokio::sync::Mutex`, preventing ratchet desync under
    /// concurrent sends to the same peer. When `None`, falls back to the
    /// legacy unsynchronized path (test fixtures + the historical sync
    /// API). Production callers must enable via [`Self::with_session_cache`].
    cache: Option<Arc<SessionCache>>,
}

/// Adapter exposing a sync [`SessionStore`] as the async
/// [`SessionPersistence`] trait the cache wants. Both store functions
/// are sync and never block on I/O (the concrete impls write to
/// in-memory parking_lot mutexes + the vault SQLite which is local),
/// so calling them from an async fn without `spawn_blocking` is sound.
struct SessionStoreAdapter(Arc<dyn SessionStore>);

#[async_trait::async_trait]
impl SessionPersistence for SessionStoreAdapter {
    async fn load(&self, peer_hex: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        self.0.load_session(peer_hex)
    }
    async fn store(&self, peer_hex: &str, session: &[u8]) -> Result<(), CryptoError> {
        self.0.store_session(peer_hex, session)
    }
}

// The ratchet state and stepping logic live in
// [`crate::signal::ratchet`] — THE Double Ratchet core shared by every
// track. This manager only owns session establishment (PQXDH) and the
// store/cache plumbing around it.

impl SignalSessionManager {
    /// Create a new session manager with the given storage backends.
    pub fn new(
        identity: Box<dyn IdentityKeyStore>,
        prekeys: Box<dyn PreKeyStore>,
        sessions: Box<dyn SessionStore>,
    ) -> Self {
        // Convert Box→Arc so we can hand the same backing store to the
        // optional cache adapter. Box→Arc::from preserves the trait
        // object's vtable; no heap re-allocation.
        let sessions: Arc<dyn SessionStore> = Arc::from(sessions);
        Self {
            identity,
            prekeys,
            sessions,
            cache: None,
        }
    }

    /// Phase 6 — enable the per-peer session cache. Builds an in-memory
    /// LRU of `peer → Arc<tokio::sync::Mutex<SessionBytes>>` backed by
    /// the manager's existing [`SessionStore`]. Once enabled,
    /// concurrent encrypts/decrypts to the SAME peer serialize on the
    /// per-peer mutex (preventing ratchet desync); encrypts to
    /// DIFFERENT peers run in parallel.
    ///
    /// `capacity` bounds the in-memory cache; evictions don't affect
    /// persistence (the underlying SessionStore is the source of truth).
    /// 256 is a reasonable default for a friend-list-scale chat client.
    #[must_use]
    pub fn with_session_cache(mut self, capacity: usize) -> Self {
        let adapter: Arc<dyn SessionPersistence> =
            Arc::new(SessionStoreAdapter(Arc::clone(&self.sessions)));
        self.cache = Some(Arc::new(SessionCache::new(adapter, capacity)));
        self
    }
}
