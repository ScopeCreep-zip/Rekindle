//! Signal Protocol session management — PQXDH + shared Double Ratchet.
//!
//! Phase 3b of the decomposed-harvest plan replaced classical X3DH with
//! PQXDH for the daemon-track Signal subsystem. The handshake primitives
//! AND the Double Ratchet stepping both come from `rekindle-crypto`
//! (`signal::pqxdh` and `signal::ratchet`) — this file used to carry its
//! own copy of the ratchet, wire-compatible with the desktop track only
//! by hand (and in fact diverged: different header, different key
//! schedule). Cross-track compatibility now holds by construction.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rekindle_crypto::signal::pqxdh::{
    self, verify::pq_signing_payload, verify::spk_signing_payload,
};
use rekindle_crypto::signal::ratchet::{self, RatchetState};
use rekindle_secrets::pq_keys::MlKemSecret;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use crate::crypto::prekeys::PreKeyBundle;
use crate::crypto::signal_store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use crate::error::{Result, TransportError};

/// Fixed identifier for the per-identity ML-KEM-768 last-resort key.
pub const PQ_LR_ID: u32 = 0;

/// Re-exported from `rekindle-crypto` — the single definition shared by
/// both tracks (the fields were already identical).
pub use rekindle_crypto::signal::SessionInitInfo;

/// Manages Signal Protocol sessions for 1:1 encrypted messaging.
pub struct SignalSessionManager {
    identity: Box<dyn IdentityKeyStore>,
    prekeys: Box<dyn PreKeyStore>,
    sessions: Box<dyn SessionStore>,
}

/// Map a shared-core crypto error onto the transport error type.
fn crypto_err(e: &rekindle_crypto::error::CryptoError) -> TransportError {
    TransportError::Internal(e.to_string())
}

impl SignalSessionManager {
    pub fn new(
        identity: Box<dyn IdentityKeyStore>,
        prekeys: Box<dyn PreKeyStore>,
        sessions: Box<dyn SessionStore>,
    ) -> Self {
        Self {
            identity,
            prekeys,
            sessions,
        }
    }

    /// Establish a session with a peer using their PreKeyBundle (initiator X3DH).
    pub fn establish_session(
        &self,
        peer_address: &str,
        bundle: &PreKeyBundle,
    ) -> Result<SessionInitInfo> {
        // PQXDH initiator (Phase 3b) — daemon-track mirror of
        // `rekindle_crypto::signal::session::SignalSessionManager::establish_session`.
        let (identity_private, _) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(&to_32(&identity_private, "identity key")?);
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        let their_ik_ed = VerifyingKey::from_bytes(&to_32(&bundle.identity_key, "their identity")?)
            .map_err(|e| TransportError::Internal(format!("their identity not on curve: {e}")))?;

        let hs = pqxdh::pqxdh_initiator(&our_ik_x25519, bundle, &their_ik_ed)
            .map_err(|e| TransportError::Internal(format!("PQXDH initiator: {e}")))?;

        // Seed the shared Double Ratchet core as initiator. `ek_secret`
        // (not the public!) seeds our ratchet secret so the responder's
        // first reply can complete the mirrored DH step.
        let okm = ratchet::expand_pqxdh_root(&hs.root_key).map_err(|e| crypto_err(&e))?;
        let session = RatchetState::initiator(&okm, *hs.ek_secret, bundle.signed_prekey.clone());
        self.sessions
            .store_session(peer_address, &session.serialize())?;
        self.identity
            .save_identity(peer_address, &bundle.identity_key)?;

        Ok(SessionInitInfo {
            ephemeral_public_key: hs.ek_public.to_vec(),
            signed_prekey_id: 1,
            one_time_prekey_id: hs.used_ot_opk_id,
            ml_kem_ciphertext: hs.ml_kem_ct,
            used_ot_pqpk_id: hs.used_ot_pqpk_id,
        })
    }

    /// Respond to a session initiated by a peer (PQXDH responder).
    pub fn respond_to_session(
        &self,
        peer_address: &str,
        their_identity_key: &[u8],
        their_ephemeral_key: &[u8],
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        ml_kem_ciphertext: &[u8],
        used_ot_pqpk_id: Option<u32>,
    ) -> Result<()> {
        let (identity_private, _) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(&to_32(&identity_private, "identity key")?);
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        let spk_data = self
            .prekeys
            .load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| TransportError::Internal("signed prekey not found".into()))?;
        let our_spk_secret = StaticSecret::from(to_32(&spk_data, "signed prekey")?);

        let our_opk_secret = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self
                .prekeys
                .load_prekey(otpk_id)?
                .ok_or_else(|| TransportError::Internal("one-time prekey not found".into()))?;
            Some(StaticSecret::from(to_32(&otpk_data, "one-time prekey")?))
        } else {
            None
        };

        let (pq_kind, pq_id) = match used_ot_pqpk_id {
            Some(id) => (PqKeyKind::OneTime, id),
            None => (PqKeyKind::LastResort, PQ_LR_ID),
        };
        let pq_secret_bytes = self
            .prekeys
            .load_pq_secret(pq_id, pq_kind)?
            .ok_or_else(|| {
                TransportError::Internal(format!(
                    "ML-KEM secret not found for ({pq_id}, {pq_kind:?})"
                ))
            })?;
        let our_ml_kem_secret = MlKemSecret::from_secret_bytes(&pq_secret_bytes)
            .ok_or_else(|| TransportError::Internal("ML-KEM secret wrong length".into()))?;

        let initiator_ik_ed =
            VerifyingKey::from_bytes(&to_32(their_identity_key, "their identity")?).map_err(
                |e| TransportError::Internal(format!("their identity not on curve: {e}")),
            )?;

        let root_key_z = pqxdh::pqxdh_responder(&pqxdh::ResponderInput {
            our_ik_x25519_secret: &our_ik_x25519,
            our_spk_secret: &our_spk_secret,
            our_opk_secret: our_opk_secret.as_ref(),
            our_ml_kem_secret: &our_ml_kem_secret,
            initiator_ik_ed: &initiator_ik_ed,
            initiator_ek_public: their_ephemeral_key,
            ml_kem_ciphertext,
        })
        .map_err(|e| TransportError::Internal(format!("PQXDH responder: {e}")))?;

        if pq_kind == PqKeyKind::OneTime {
            self.prekeys.remove_pq_secret(pq_id, PqKeyKind::OneTime)?;
        }
        if let Some(otpk_id) = one_time_prekey_id {
            self.prekeys.remove_prekey(otpk_id)?;
        }

        // Seed the shared Double Ratchet core as responder (chain
        // assignment mirrors the initiator's). The SPK SECRET seeds our
        // ratchet secret — the initiator's first message DHs against it.
        let okm = ratchet::expand_pqxdh_root(&root_key_z).map_err(|e| crypto_err(&e))?;
        let session = RatchetState::responder(
            &okm,
            our_spk_secret.to_bytes(),
            their_ephemeral_key.to_vec(),
        );
        self.sessions
            .store_session(peer_address, &session.serialize())?;
        self.identity
            .save_identity(peer_address, their_identity_key)?;
        Ok(())
    }

    /// Encrypt a plaintext message for a peer with an established session.
    ///
    /// Performs a DH ratchet step on every message: generates a new ephemeral
    /// keypair, performs DH with the peer's last ratchet public key, derives
    /// new root + chain keys. The new ratchet public key is included in the
    /// message so the receiver can perform the corresponding ratchet step.
    ///
    /// Wire format: `[ratchet_public(32) || counter(8 LE) || nonce(12) || ciphertext+tag]`
    pub fn encrypt(&self, peer_address: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let session_data = self.sessions.load_session(peer_address)?.ok_or_else(|| {
            TransportError::Internal(format!("no Signal session for {peer_address}"))
        })?;
        let mut session = RatchetState::deserialize(&session_data).map_err(|e| crypto_err(&e))?;
        let output = session
            .encrypt_step(plaintext)
            .map_err(|e| crypto_err(&e))?;
        self.sessions
            .store_session(peer_address, &session.serialize())?;
        Ok(output)
    }

    /// Decrypt a ciphertext message from a peer.
    ///
    /// Performs the receiver-side DH ratchet step: extracts the sender's new
    /// ratchet public key from the message, performs DH with our ratchet secret,
    /// derives new root + receiving chain keys.
    ///
    /// Wire format: `[ratchet_public(32) || counter(8 LE) || nonce(12) || ciphertext+tag]`
    pub fn decrypt(&self, peer_address: &str, message: &[u8]) -> Result<Vec<u8>> {
        let session_data = self.sessions.load_session(peer_address)?.ok_or_else(|| {
            TransportError::Internal(format!("no Signal session for {peer_address}"))
        })?;
        let mut session = RatchetState::deserialize(&session_data).map_err(|e| crypto_err(&e))?;
        let plaintext = session.decrypt_step(message).map_err(|e| crypto_err(&e))?;
        self.sessions
            .store_session(peer_address, &session.serialize())?;
        Ok(plaintext)
    }

    /// Check if a session exists with a peer.
    pub fn has_session(&self, peer_address: &str) -> Result<bool> {
        self.sessions.has_session(peer_address)
    }

    /// Delete session with a peer.
    pub fn delete_session(&self, peer_address: &str) -> Result<()> {
        self.sessions.delete_session(peer_address)
    }

    /// Load a signed prekey's private key bytes from the store.
    ///
    /// Used by the identity ceremony to extract prekey material for
    /// persistence to the OS keyring.
    pub fn load_signed_prekey(&self, id: u32) -> Result<Vec<u8>> {
        self.prekeys
            .load_signed_prekey(id)?
            .ok_or_else(|| TransportError::Internal(format!("signed prekey {id} not found")))
    }

    /// Load a one-time prekey's private key bytes from the store.
    ///
    /// Used by the identity ceremony to extract prekey material for
    /// persistence to the OS keyring.
    pub fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        self.prekeys.load_prekey(id)
    }

    /// Generate a PreKeyBundle for publication to DHT (PQXDH-augmented).
    pub fn generate_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        pq_one_time_id: Option<u32>,
    ) -> Result<PreKeyBundle> {
        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;
        let signing_key =
            SigningKey::from_bytes(&to_32(&identity_private, "identity for signing")?);

        // X25519 signed prekey.
        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekeys
            .store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;
        let signed_prekey_signature = signing_key
            .sign(&spk_signing_payload(signed_prekey_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Optional X25519 one-time prekey.
        let one_time_prekey = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
            let otpk_public = X25519Public::from(&otpk_secret);
            self.prekeys.store_prekey(otpk_id, otpk_secret.as_bytes())?;
            Some(otpk_public.as_bytes().to_vec())
        } else {
            None
        };

        // ML-KEM-768 last-resort key (singleton, PQ_LR_ID).
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

        // Optional ML-KEM-768 one-time key.
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

// ── Helpers ─────────────────────────────────────────────────────────────

fn to_32(data: &[u8], label: &str) -> Result<[u8; 32]> {
    data[..32].try_into().map_err(|_| {
        TransportError::Internal(format!("{label}: expected 32 bytes, got {}", data.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::signal_store::{MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore};
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct SharedSessionStore(Arc<Mutex<HashMap<String, Vec<u8>>>>);

    impl SessionStore for SharedSessionStore {
        fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
            Ok(self.0.lock().get(address).cloned())
        }
        fn store_session(&self, address: &str, data: &[u8]) -> Result<()> {
            self.0.lock().insert(address.to_string(), data.to_vec());
            Ok(())
        }
        fn has_session(&self, address: &str) -> Result<bool> {
            Ok(self.0.lock().contains_key(address))
        }
        fn delete_session(&self, address: &str) -> Result<()> {
            self.0.lock().remove(address);
            Ok(())
        }
        fn list_sessions(&self) -> Result<Vec<String>> {
            Ok(self.0.lock().keys().cloned().collect())
        }
    }

    /// Ed25519 identity keypair — bytes match production layout
    /// (`MemoryIdentityStore` holds the Ed25519 32-byte secret + Ed25519
    /// 32-byte public). PQXDH derives X25519 from Ed25519 internally via
    /// `to_scalar_bytes()`, matching `Identity::to_x25519_secret`.
    fn make_identity() -> (Vec<u8>, Vec<u8>) {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying = signing.verifying_key();
        (signing.to_bytes().to_vec(), verifying.to_bytes().to_vec())
    }

    fn establish_pair() -> (SignalSessionManager, SignalSessionManager, String, String) {
        let (alice_priv, alice_pub) = make_identity();
        let (bob_priv, bob_pub) = make_identity();

        let alice = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        );
        let bob = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(bob_priv, bob_pub.clone(), 2)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        );

        let alice_addr = hex::encode(&alice_pub);
        let bob_addr = hex::encode(&bob_pub);

        let bob_bundle = bob.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
        let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();

        bob.respond_to_session(
            &alice_addr,
            &alice_pub,
            &init.ephemeral_public_key,
            init.signed_prekey_id,
            init.one_time_prekey_id,
            &init.ml_kem_ciphertext,
            init.used_ot_pqpk_id,
        )
        .unwrap();

        (alice, bob, alice_addr, bob_addr)
    }

    #[test]
    fn x3dh_encrypt_decrypt_roundtrip() {
        let (alice, bob, alice_addr, bob_addr) = establish_pair();

        let ct = alice.encrypt(&bob_addr, b"hello bob").unwrap();
        let pt = bob.decrypt(&alice_addr, &ct).unwrap();
        assert_eq!(pt, b"hello bob");

        let reply_ct = bob.encrypt(&alice_addr, b"hi alice").unwrap();
        let reply_pt = alice.decrypt(&bob_addr, &reply_ct).unwrap();
        assert_eq!(reply_pt, b"hi alice");
    }

    #[test]
    fn multiple_messages_different_ciphertexts() {
        let (alice, bob, alice_addr, bob_addr) = establish_pair();

        let c1 = alice.encrypt(&bob_addr, b"msg1").unwrap();
        let c2 = alice.encrypt(&bob_addr, b"msg2").unwrap();
        assert_ne!(c1, c2);

        assert_eq!(bob.decrypt(&alice_addr, &c1).unwrap(), b"msg1");
        assert_eq!(bob.decrypt(&alice_addr, &c2).unwrap(), b"msg2");
    }

    #[test]
    fn prekey_bundle_generation() {
        let (priv_key, pub_key) = make_identity();
        let mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );

        let bundle = mgr.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
        assert_eq!(bundle.signed_prekey.len(), 32);
        assert_eq!(bundle.signed_prekey_signature.len(), 64);
        assert!(bundle.one_time_prekey.is_some());
        assert_eq!(bundle.registration_id, 1);
        // PQXDH additions: ML-KEM-768 last-resort bundle is always present.
        assert_eq!(bundle.pqpk_lr.len(), 1184);
        assert_eq!(bundle.pqpk_lr_signature.len(), 64);
        assert!(bundle.pqpk_ot.is_some());
    }

    #[test]
    fn encrypt_without_session_fails() {
        let (priv_key, pub_key) = make_identity();
        let mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );
        assert!(mgr.encrypt("nobody", b"hello").is_err());
    }

    /// CROSS-TRACK INTEROP — the reason the shared ratchet core exists.
    ///
    /// Alice runs the daemon track's manager (this file); Bob runs the
    /// desktop track's (`rekindle_crypto::signal::session`). Before the
    /// ratchets were converged this could not work: the desktop wrote
    /// `[counter || nonce || ct]` with a symmetric-only chain while the
    /// daemon wrote `[ratchet_public || counter || nonce || ct]` with a
    /// per-message DH step — a DM between the two tracks could not
    /// decrypt. This test is the regression guard that keeps the tracks
    /// on ONE wire format and ONE key schedule.
    #[tokio::test]
    async fn cross_track_session_interop() {
        use rekindle_crypto::signal::memory_stores as desktop_stores;
        use rekindle_crypto::signal::session::SignalSessionManager as DesktopManager;

        let (alice_priv, alice_pub) = make_identity();
        let (bob_priv, bob_pub) = make_identity();

        // Alice: daemon-track manager (transport stores).
        let alice = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );
        // Bob: desktop-track manager (rekindle-crypto stores).
        let bob = DesktopManager::new(
            Box::new(desktop_stores::MemoryIdentityStore::new(
                bob_priv,
                bob_pub.clone(),
                2,
            )),
            Box::new(desktop_stores::MemoryPreKeyStore::new()),
            Box::new(desktop_stores::MemorySessionStore::new()),
        );

        let alice_addr = hex::encode(&alice_pub);
        let bob_addr = hex::encode(&bob_pub);

        // Bob (desktop) publishes a bundle; Alice (daemon) initiates.
        let bob_bundle = bob.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
        let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();
        bob.respond_to_session(
            &alice_addr,
            &alice_pub,
            &init.ephemeral_public_key,
            init.signed_prekey_id,
            init.one_time_prekey_id,
            &init.ml_kem_ciphertext,
            init.used_ot_pqpk_id,
        )
        .unwrap();

        // Daemon → desktop.
        let wire = alice
            .encrypt(&bob_addr, b"hello from the daemon track")
            .unwrap();
        assert_eq!(
            bob.decrypt(&alice_addr, &wire).await.unwrap(),
            b"hello from the daemon track"
        );

        // Desktop → daemon.
        let wire = bob
            .encrypt(&alice_addr, b"hello from the desktop track")
            .await
            .unwrap();
        assert_eq!(
            alice.decrypt(&bob_addr, &wire).unwrap(),
            b"hello from the desktop track"
        );

        // A short ping-pong to prove the DH ratchet stays in step
        // across implementations, not just on the first exchange.
        for i in 0..4u8 {
            let m = vec![i; 32];
            if i % 2 == 0 {
                let w = alice.encrypt(&bob_addr, &m).unwrap();
                assert_eq!(bob.decrypt(&alice_addr, &w).await.unwrap(), m);
            } else {
                let w = bob.encrypt(&alice_addr, &m).await.unwrap();
                assert_eq!(alice.decrypt(&bob_addr, &w).unwrap(), m);
            }
        }
    }

    /// Responder-replies-first also works cross-track — this ordering
    /// was broken in BOTH pre-convergence forks (each stored the
    /// initiator's ephemeral PUBLIC key where the ratchet secret
    /// belongs, so the initiator could never decrypt a first inbound).
    #[tokio::test]
    async fn cross_track_responder_sends_first() {
        use rekindle_crypto::signal::memory_stores as desktop_stores;
        use rekindle_crypto::signal::session::SignalSessionManager as DesktopManager;

        let (alice_priv, alice_pub) = make_identity();
        let (bob_priv, bob_pub) = make_identity();

        let alice = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );
        let bob = DesktopManager::new(
            Box::new(desktop_stores::MemoryIdentityStore::new(
                bob_priv,
                bob_pub.clone(),
                2,
            )),
            Box::new(desktop_stores::MemoryPreKeyStore::new()),
            Box::new(desktop_stores::MemorySessionStore::new()),
        );

        let alice_addr = hex::encode(&alice_pub);
        let bob_addr = hex::encode(&bob_pub);

        let bob_bundle = bob.generate_prekey_bundle(1, None, None).unwrap();
        let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();
        bob.respond_to_session(
            &alice_addr,
            &alice_pub,
            &init.ephemeral_public_key,
            init.signed_prekey_id,
            init.one_time_prekey_id,
            &init.ml_kem_ciphertext,
            init.used_ot_pqpk_id,
        )
        .unwrap();

        // Bob (the responder) speaks FIRST.
        let wire = bob.encrypt(&alice_addr, b"responder first").await.unwrap();
        assert_eq!(alice.decrypt(&bob_addr, &wire).unwrap(), b"responder first");
    }
}
