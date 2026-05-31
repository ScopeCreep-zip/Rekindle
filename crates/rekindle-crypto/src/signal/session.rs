use std::sync::Arc;

use crate::error::CryptoError;
use crate::signal::pqxdh::{self, verify::pq_signing_payload, verify::spk_signing_payload};
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::session_cache::{SessionCache, SessionPersistence};
use crate::signal::store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rekindle_secrets::pq_keys::MlKemSecret;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::ZeroizeOnDrop;

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
/// Uses X3DH for session establishment and a simplified Double Ratchet
/// for forward-secret message encryption.
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

/// An established session's symmetric ratchet state. Architecture
/// §32 line 4138 mandates `ZeroizeOnDrop` on every secret type — root,
/// chain, and ratchet keys are scrubbed from memory when the state is
/// dropped (session close / Drop on session-store eviction). The
/// `their_ratchet_public` and counters are non-secret so they can be
/// excluded from zeroize via `#[zeroize(skip)]`.
#[derive(Clone, ZeroizeOnDrop)]
struct RatchetState {
    /// Root key — evolves with each DH ratchet step.
    root_key: [u8; 32],
    /// Sending chain key — evolves with each message sent.
    sending_chain_key: [u8; 32],
    /// Receiving chain key — evolves with each message received.
    receiving_chain_key: [u8; 32],
    /// Our current DH ratchet keypair (X25519).
    our_ratchet_secret: Vec<u8>,
    /// Their current DH ratchet public key.
    #[zeroize(skip)]
    their_ratchet_public: Vec<u8>,
    /// Send message counter.
    #[zeroize(skip)]
    send_counter: u64,
    /// Receive message counter.
    #[zeroize(skip)]
    recv_counter: u64,
}

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

    /// Establish a session with a peer using their `PreKeyBundle` (X3DH).
    ///
    /// This is the initiator side — called when we want to start a conversation
    /// with someone whose `PreKeyBundle` we fetched from DHT.
    pub fn establish_session(
        &self,
        peer_address: &str,
        bundle: &PreKeyBundle,
    ) -> Result<SessionInitInfo, CryptoError> {
        // PQXDH initiator (Phase 3b): replaces the classical X3DH body.
        // The Ed25519 identity is converted to X25519 form via the
        // standard scalar derivation; PQ keys are verified, ML-KEM
        // ciphertext is encapsulated, root_key is derived via the
        // PQXDH KDF (F || DH1 || DH2 || DH3 [|| DH4] || SS).

        // 1. Our X25519 identity secret (Ed25519 → X25519 via scalar).
        let (identity_private, _identity_public) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(
            &<[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length".into()))?,
        );
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        // 2. Their Ed25519 identity (verifying key) — used for signature
        //    verification on SPK and PQ keys.
        let their_ik_ed = VerifyingKey::from_bytes(
            &<[u8; 32]>::try_from(bundle.identity_key.as_slice())
                .map_err(|_| CryptoError::InvalidKey("their identity key wrong length".into()))?,
        )
        .map_err(|e| CryptoError::InvalidKey(format!("their identity key not on curve: {e}")))?;

        // 3. Run the PQXDH initiator handshake against the bundle.
        let hs = pqxdh::pqxdh_initiator(&our_ik_x25519, bundle, &their_ik_ed)
            .map_err(|e| CryptoError::SessionError(format!("PQXDH initiator: {e}")))?;

        // 4. Expand the PQXDH root_key into sending + receiving chain
        //    keys for the Double Ratchet. The expansion mirrors the
        //    legacy X3DH HKDF split, just with PQXDH's root_key as the
        //    starting material.
        let hk = Hkdf::<Sha256>::new(None, &*hs.root_key);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindlePQXDH", &mut okm)
            .map_err(|e| CryptoError::SessionError(format!("HKDF expand failed: {e}")))?;
        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        sending_chain_key.copy_from_slice(&okm[32..64]);
        receiving_chain_key.copy_from_slice(&okm[64..96]);

        // 5. Persist the initial ratchet state.
        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: hs.ek_public.to_vec(),
            their_ratchet_public: bundle.signed_prekey.clone(),
            send_counter: 0,
            recv_counter: 0,
        };
        let session_data = serialize_ratchet(&ratchet);
        self.sessions.store_session(peer_address, &session_data)?;

        // Trust their identity on first use (TOFU)
        self.identity
            .save_identity(peer_address, &bundle.identity_key)?;

        Ok(SessionInitInfo {
            ephemeral_public_key: hs.ek_public.to_vec(),
            // SPK id 1 matches generate_prekey_bundle(1, ...) convention.
            signed_prekey_id: 1,
            one_time_prekey_id: hs.used_ot_opk_id,
            ml_kem_ciphertext: hs.ml_kem_ct,
            used_ot_pqpk_id: hs.used_ot_pqpk_id,
        })
    }

    /// Respond to a session initiated by a peer (responder-side X3DH).
    ///
    /// Called when we receive a friend request or initial message containing
    /// the initiator's identity key and ephemeral public key.
    pub fn respond_to_session(
        &self,
        peer_address: &str,
        their_identity_key: &[u8],
        their_ephemeral_key: &[u8],
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        ml_kem_ciphertext: &[u8],
        used_ot_pqpk_id: Option<u32>,
    ) -> Result<(), CryptoError> {
        // PQXDH responder (Phase 3b): mirror of `establish_session`.
        // Loads our X25519 identity, signed prekey, optional OPK, and
        // the ML-KEM secret matching whichever PQ key the initiator
        // encapsulated to. Reconstructs the same root_key.

        // 1. Our X25519 identity (Ed25519 scalar form).
        let (identity_private, _identity_public) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(
            &<[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length".into()))?,
        );
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        // 2. Our signed prekey secret.
        let spk_data = self
            .prekeys
            .load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| CryptoError::InvalidKey("signed prekey not found".into()))?;
        let our_spk_secret = StaticSecret::from(
            <[u8; 32]>::try_from(spk_data.as_slice())
                .map_err(|_| CryptoError::InvalidKey("signed prekey wrong length".into()))?,
        );

        // 3. Our optional one-time prekey secret.
        let our_opk_secret = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self
                .prekeys
                .load_prekey(otpk_id)?
                .ok_or_else(|| CryptoError::InvalidKey("one-time prekey not found".into()))?;
            Some(StaticSecret::from(
                <[u8; 32]>::try_from(otpk_data.as_slice())
                    .map_err(|_| CryptoError::InvalidKey("one-time prekey wrong length".into()))?,
            ))
        } else {
            None
        };

        // 4. Our ML-KEM secret (one-time preferred; else last-resort).
        let (pq_kind, pq_id) = match used_ot_pqpk_id {
            Some(id) => (PqKeyKind::OneTime, id),
            None => (PqKeyKind::LastResort, PQ_LR_ID),
        };
        let pq_secret_bytes = self
            .prekeys
            .load_pq_secret(pq_id, pq_kind)?
            .ok_or_else(|| {
                CryptoError::InvalidKey(format!(
                    "ML-KEM secret not found for ({pq_id}, {pq_kind:?})"
                ))
            })?;
        let our_ml_kem_secret = MlKemSecret::from_secret_bytes(&pq_secret_bytes)
            .ok_or_else(|| CryptoError::InvalidKey("ML-KEM secret wrong length".into()))?;

        // 5. Initiator's Ed25519 identity (for X25519 DH partner derivation).
        let initiator_ik_ed = VerifyingKey::from_bytes(
            &<[u8; 32]>::try_from(their_identity_key)
                .map_err(|_| CryptoError::InvalidKey("their identity key wrong length".into()))?,
        )
        .map_err(|e| CryptoError::InvalidKey(format!("their identity key not on curve: {e}")))?;

        // 6. Run the PQXDH responder.
        let root_key_z = pqxdh::pqxdh_responder(&pqxdh::ResponderInput {
            our_ik_x25519_secret: &our_ik_x25519,
            our_spk_secret: &our_spk_secret,
            our_opk_secret: our_opk_secret.as_ref(),
            our_ml_kem_secret: &our_ml_kem_secret,
            initiator_ik_ed: &initiator_ik_ed,
            initiator_ek_public: their_ephemeral_key,
            ml_kem_ciphertext,
        })
        .map_err(|e| CryptoError::SessionError(format!("PQXDH responder: {e}")))?;

        // 7. Consume one-time keys (PQ OT + X25519 OPK).
        if pq_kind == PqKeyKind::OneTime {
            self.prekeys.remove_pq_secret(pq_id, PqKeyKind::OneTime)?;
        }
        if let Some(otpk_id) = one_time_prekey_id {
            self.prekeys.remove_prekey(otpk_id)?;
        }

        // 8. Expand the root_key into chain keys.
        let hk = Hkdf::<Sha256>::new(None, &*root_key_z);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindlePQXDH", &mut okm)
            .map_err(|e| CryptoError::SessionError(format!("HKDF expand failed: {e}")))?;
        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        // Responder swaps sending/receiving relative to initiator.
        receiving_chain_key.copy_from_slice(&okm[32..64]);
        sending_chain_key.copy_from_slice(&okm[64..96]);

        let spk_bytes = our_spk_secret.to_bytes();
        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: X25519Public::from(&StaticSecret::from(spk_bytes))
                .as_bytes()
                .to_vec(),
            their_ratchet_public: their_ephemeral_key.to_vec(),
            send_counter: 0,
            recv_counter: 0,
        };

        let session_data = serialize_ratchet(&ratchet);
        self.sessions.store_session(peer_address, &session_data)?;

        // Trust their identity on first use (TOFU)
        self.identity
            .save_identity(peer_address, their_identity_key)?;

        Ok(())
    }

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
        let mut ratchet = deserialize_ratchet(&guard)?;
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
        let mut ratchet = deserialize_ratchet(&session_data)?;
        let (output, new_data) = Self::encrypt_mutate(&mut ratchet, plaintext)?;
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(output)
    }

    /// Pure mutate step shared by cache and non-cache paths. Returns
    /// `(wire_output, new_session_bytes)`. Caller is responsible for
    /// persisting `new_session_bytes` (under the per-peer lock when
    /// the cache is in play).
    fn encrypt_mutate(
        ratchet: &mut RatchetState,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        // Derive message key from sending chain key via HKDF
        let hk = Hkdf::<Sha256>::new(None, &ratchet.sending_chain_key);
        let mut message_key = [0u8; 32];
        let mut next_chain_key = [0u8; 32];
        hk.expand(b"ReKindleMsgKey", &mut message_key)
            .map_err(|e| CryptoError::EncryptionError(format!("HKDF: {e}")))?;
        hk.expand(b"ReKindleChainKey", &mut next_chain_key)
            .map_err(|e| CryptoError::EncryptionError(format!("HKDF: {e}")))?;

        // Advance sending chain
        ratchet.sending_chain_key = next_chain_key;
        ratchet.send_counter += 1;

        // Encrypt with AES-256-GCM
        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&ratchet.send_counter.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        // Prepend counter + nonce for the recipient
        let mut output = Vec::with_capacity(8 + 12 + ciphertext.len());
        output.extend_from_slice(&ratchet.send_counter.to_le_bytes());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        let new_session_data = serialize_ratchet(ratchet);
        Ok((output, new_session_data))
    }

    /// Decrypt a ciphertext message from a peer.
    ///
    /// Phase 6 — see [`Self::encrypt`]. Same cache-vs-fallback semantics.
    pub async fn decrypt(
        &self,
        peer_address: &str,
        message: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        if message.len() < 20 {
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
        let mut ratchet = deserialize_ratchet(&guard)?;
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
        let mut ratchet = deserialize_ratchet(&session_data)?;
        let (plaintext, new_data) = Self::decrypt_mutate(&mut ratchet, message)?;
        self.sessions.store_session(peer_address, &new_data)?;
        Ok(plaintext)
    }

    fn decrypt_mutate(
        ratchet: &mut RatchetState,
        message: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        // Parse counter + nonce + ciphertext
        let _counter = u64::from_le_bytes(
            message[..8]
                .try_into()
                .map_err(|_| CryptoError::DecryptionError("invalid counter".into()))?,
        );
        let nonce_bytes: [u8; 12] = message[8..20]
            .try_into()
            .map_err(|_| CryptoError::DecryptionError("invalid nonce".into()))?;
        let ciphertext = &message[20..];

        // Derive message key from receiving chain key
        let hk = Hkdf::<Sha256>::new(None, &ratchet.receiving_chain_key);
        let mut message_key = [0u8; 32];
        let mut next_chain_key = [0u8; 32];
        hk.expand(b"ReKindleMsgKey", &mut message_key)
            .map_err(|e| CryptoError::DecryptionError(format!("HKDF: {e}")))?;
        hk.expand(b"ReKindleChainKey", &mut next_chain_key)
            .map_err(|e| CryptoError::DecryptionError(format!("HKDF: {e}")))?;

        // Advance receiving chain
        ratchet.receiving_chain_key = next_chain_key;
        ratchet.recv_counter += 1;

        // Decrypt with AES-256-GCM
        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;

        let new_session_data = serialize_ratchet(ratchet);
        Ok((plaintext, new_session_data))
    }

    /// Check if we have an established session with a peer.
    pub fn has_session(&self, peer_address: &str) -> Result<bool, CryptoError> {
        self.sessions.has_session(peer_address)
    }

    /// TOFU check: whether the given `identity_key` matches the trusted
    /// record for `peer_address`. Returns `Ok(true)` for a first-contact
    /// peer (no stored record) — that's the trust-on-first-use semantics
    /// per `IdentityKeyStore.java:54-60` (libsignal). Returns `Ok(false)`
    /// only when there IS a stored record AND the keys disagree —
    /// signaling that the peer's identity rotated (legitimate re-onboard
    /// or substitution attack; either way the application layer should
    /// require explicit user consent before wiping the existing session).
    ///
    /// W16.10d follow-up — exposes the underlying [`IdentityKeyStore`]
    /// trait method so callers don't have to thread the store separately.
    pub fn is_trusted_identity(
        &self,
        peer_address: &str,
        identity_key: &[u8],
    ) -> Result<bool, CryptoError> {
        self.identity
            .is_trusted_identity(peer_address, identity_key)
    }

    /// Delete an existing session with a peer (e.g., on friend removal).
    pub fn delete_session(&self, peer_address: &str) -> Result<(), CryptoError> {
        self.sessions.delete_session(peer_address)
    }

    /// P1.2 — load an existing `PreKeyBundle` from the persisted prekey
    /// store WITHOUT regenerating keys.
    ///
    /// Returns `Ok(Some(bundle))` when both the signed prekey
    /// (`signed_prekey_id`) and — if requested — the one-time prekey
    /// (`one_time_prekey_id`) are already in the store. Reconstructs
    /// the public side from the stored X25519 secret and re-signs the
    /// signed-prekey public bytes with the identity key.
    ///
    /// Returns `Ok(None)` when any required prekey is missing — caller
    /// should call `generate_prekey_bundle` to mint fresh keys.
    ///
    /// **Why this exists**: a Stronghold-backed prekey store survives
    /// restart; calling `generate_prekey_bundle` unconditionally on
    /// every login overwrites prekey #1 + signed_prekey #1 in
    /// Stronghold AND publishes a fresh bundle to DHT subkey 5,
    /// breaking peers' cached PreKeyBundles and any in-flight messages
    /// encrypted to the previous bundle. This method gives callers a
    /// "use existing if present" path so steady-state logins reuse the
    /// already-published bundle.
    pub fn load_existing_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        pq_one_time_id: Option<u32>,
    ) -> Result<Option<PreKeyBundle>, CryptoError> {
        let Some(signed_prekey_secret_bytes) = self.prekeys.load_signed_prekey(signed_prekey_id)?
        else {
            return Ok(None);
        };

        let one_time_prekey_bytes = if let Some(otpk_id) = one_time_prekey_id {
            match self.prekeys.load_prekey(otpk_id)? {
                Some(bytes) => Some(bytes),
                None => return Ok(None), // requested OTPK missing → caller mints fresh
            }
        } else {
            None
        };

        // Phase 3b — PQ last-resort key must exist; if missing, caller
        // mints fresh.
        let Some(pq_lr_bytes) = self
            .prekeys
            .load_pq_secret(PQ_LR_ID, PqKeyKind::LastResort)?
        else {
            return Ok(None);
        };
        let pq_lr_secret = MlKemSecret::from_secret_bytes(&pq_lr_bytes)
            .ok_or_else(|| CryptoError::InvalidKey("PQ LR secret wrong length".into()))?;
        let pq_lr_public = pq_lr_secret.public();

        // Phase 3b — PQ one-time key (optional).
        let pq_ot_secret_opt = if let Some(id) = pq_one_time_id {
            match self.prekeys.load_pq_secret(id, PqKeyKind::OneTime)? {
                Some(bytes) => {
                    Some(MlKemSecret::from_secret_bytes(&bytes).ok_or_else(|| {
                        CryptoError::InvalidKey("PQ OT secret wrong length".into())
                    })?)
                }
                None => return Ok(None),
            }
        } else {
            None
        };

        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        // Reconstruct the X25519 public side of the stored signed prekey.
        let secret_array: [u8; 32] = <[u8; 32]>::try_from(&signed_prekey_secret_bytes[..])
            .map_err(|_| CryptoError::InvalidKey("signed prekey wrong length".into()))?;
        let signed_prekey_secret = StaticSecret::from(secret_array);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);

        // Re-sign the public bytes with the identity key, using the
        // PQXDH domain-separated payload (0x01 || SPK).
        let signing_key =
            SigningKey::from_bytes(&<[u8; 32]>::try_from(&identity_private[..32]).map_err(
                |_| CryptoError::InvalidKey("identity key wrong length for signing".into()),
            )?);
        let signed_prekey_signature = signing_key
            .sign(&spk_signing_payload(signed_prekey_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Reconstruct one-time prekey public if present.
        let one_time_prekey = if let Some(bytes) = one_time_prekey_bytes {
            let array: [u8; 32] = <[u8; 32]>::try_from(&bytes[..])
                .map_err(|_| CryptoError::InvalidKey("one-time prekey wrong length".into()))?;
            let secret = StaticSecret::from(array);
            Some(X25519Public::from(&secret).as_bytes().to_vec())
        } else {
            None
        };

        // Re-sign PQ keys with the identity (domain-separated payloads).
        let pq_lr_signature = signing_key
            .sign(&pq_signing_payload(b"LR", pq_lr_public.as_bytes()))
            .to_bytes()
            .to_vec();
        let (pq_ot, pq_ot_signature) = match pq_ot_secret_opt {
            Some(ot_secret) => {
                let ot_public = ot_secret.public();
                let sig = signing_key
                    .sign(&pq_signing_payload(b"OT", ot_public.as_bytes()))
                    .to_bytes()
                    .to_vec();
                (Some(ot_public.as_bytes().to_vec()), Some(sig))
            }
            None => (None, None),
        };

        Ok(Some(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature,
            one_time_prekey,
            one_time_prekey_id,
            registration_id,
            pqpk_lr: pq_lr_public.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_signature,
            pqpk_ot: pq_ot,
            pqpk_ot_signature: pq_ot_signature,
            pqpk_ot_id: pq_one_time_id,
        }))
    }

    /// Generate a `PreKeyBundle` for publication to DHT.
    ///
    /// Creates a signed prekey, optional one-time prekey, mandatory PQ
    /// last-resort prekey, and optional PQ one-time prekey. Stores all
    /// secrets in the prekey store and returns the bundle.
    pub fn generate_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        pq_one_time_id: Option<u32>,
    ) -> Result<PreKeyBundle, CryptoError> {
        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        let signing_key =
            SigningKey::from_bytes(&<[u8; 32]>::try_from(&identity_private[..32]).map_err(
                |_| CryptoError::InvalidKey("identity key wrong length for signing".into()),
            )?);

        // Generate signed prekey (X25519).
        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekeys
            .store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;
        let signed_prekey_signature = signing_key
            .sign(&spk_signing_payload(signed_prekey_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Optionally generate a one-time X25519 prekey.
        let one_time_prekey = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
            let otpk_public = X25519Public::from(&otpk_secret);
            self.prekeys.store_prekey(otpk_id, otpk_secret.as_bytes())?;
            Some(otpk_public.as_bytes().to_vec())
        } else {
            None
        };

        // Generate ML-KEM-768 last-resort key (singleton, PQ_LR_ID = 0).
        let (pq_lr_secret, pq_lr_public) = MlKemSecret::generate();
        self.prekeys.store_pq_secret(
            PQ_LR_ID,
            PqKeyKind::LastResort,
            pq_lr_secret.as_secret_bytes(),
        )?;
        let pq_lr_signature = signing_key
            .sign(&pq_signing_payload(b"LR", pq_lr_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Optionally generate a one-time ML-KEM-768 prekey.
        let (pq_ot, pq_ot_signature) = if let Some(id) = pq_one_time_id {
            let (ot_secret, ot_public) = MlKemSecret::generate();
            self.prekeys
                .store_pq_secret(id, PqKeyKind::OneTime, ot_secret.as_secret_bytes())?;
            let sig = signing_key
                .sign(&pq_signing_payload(b"OT", ot_public.as_bytes()))
                .to_bytes()
                .to_vec();
            (Some(ot_public.as_bytes().to_vec()), Some(sig))
        } else {
            (None, None)
        };

        Ok(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature,
            one_time_prekey,
            one_time_prekey_id,
            registration_id,
            pqpk_lr: pq_lr_public.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_signature,
            pqpk_ot: pq_ot,
            pqpk_ot_signature: pq_ot_signature,
            pqpk_ot_id: pq_one_time_id,
        })
    }
}

// Simple binary serialization for ratchet state.
fn serialize_ratchet(state: &RatchetState) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&state.root_key);
    data.extend_from_slice(&state.sending_chain_key);
    data.extend_from_slice(&state.receiving_chain_key);
    let our_len = u32::try_from(state.our_ratchet_secret.len())
        .expect("ratchet secret length must fit in u32");
    data.extend_from_slice(&our_len.to_le_bytes());
    data.extend_from_slice(&state.our_ratchet_secret);
    let their_len = u32::try_from(state.their_ratchet_public.len())
        .expect("ratchet public length must fit in u32");
    data.extend_from_slice(&their_len.to_le_bytes());
    data.extend_from_slice(&state.their_ratchet_public);
    data.extend_from_slice(&state.send_counter.to_le_bytes());
    data.extend_from_slice(&state.recv_counter.to_le_bytes());
    data
}

fn deserialize_ratchet(data: &[u8]) -> Result<RatchetState, CryptoError> {
    if data.len() < 112 {
        return Err(CryptoError::SessionError("invalid session data".into()));
    }

    let mut pos = 0;

    let mut root_key = [0u8; 32];
    root_key.copy_from_slice(&data[pos..pos + 32]);
    pos += 32;

    let mut sending_chain_key = [0u8; 32];
    sending_chain_key.copy_from_slice(&data[pos..pos + 32]);
    pos += 32;

    let mut receiving_chain_key = [0u8; 32];
    receiving_chain_key.copy_from_slice(&data[pos..pos + 32]);
    pos += 32;

    let our_len = usize::try_from(u32::from_le_bytes(
        data[pos..pos + 4]
            .try_into()
            .map_err(|_| CryptoError::SessionError("corrupt session".into()))?,
    ))
    .map_err(|_| CryptoError::SessionError("ratchet secret length overflow".into()))?;
    pos += 4;
    let our_ratchet_secret = data[pos..pos + our_len].to_vec();
    pos += our_len;

    let their_len = usize::try_from(u32::from_le_bytes(
        data[pos..pos + 4]
            .try_into()
            .map_err(|_| CryptoError::SessionError("corrupt session".into()))?,
    ))
    .map_err(|_| CryptoError::SessionError("ratchet public length overflow".into()))?;
    pos += 4;
    let their_ratchet_public = data[pos..pos + their_len].to_vec();
    pos += their_len;

    let send_counter = u64::from_le_bytes(
        data[pos..pos + 8]
            .try_into()
            .map_err(|_| CryptoError::SessionError("corrupt session".into()))?,
    );
    pos += 8;

    let recv_counter = u64::from_le_bytes(
        data[pos..pos + 8]
            .try_into()
            .map_err(|_| CryptoError::SessionError("corrupt session".into()))?,
    );

    Ok(RatchetState {
        root_key,
        sending_chain_key,
        receiving_chain_key,
        our_ratchet_secret,
        their_ratchet_public,
        send_counter,
        recv_counter,
    })
}
