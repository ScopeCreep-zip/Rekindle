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
use rekindle_crypto::bytes::to_32;
use rekindle_crypto::signal::pqxdh::{
    self, verify::pq_signing_payload, verify::spk_signing_payload,
};
use rekindle_crypto::signal::ratchet::{self, RatchetState};
use rekindle_secrets::pq_keys::MlKemSecret;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use crate::crypto::prekeys::PreKeyBundle;
use crate::crypto::signal_store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use crate::error::{Result, TransportError};

/// Re-exported from `rekindle-crypto` — the single definitions shared
/// by both tracks. `PQ_LR_ID` was previously redeclared here, two lines
/// above the re-export that already established the pattern.
pub use rekindle_crypto::signal::session::PQ_LR_ID;
pub use rekindle_crypto::signal::SessionInitInfo;

/// Manages Signal Protocol sessions for 1:1 encrypted messaging.
pub struct SignalSessionManager {
    identity: Box<dyn IdentityKeyStore>,
    prekeys: Box<dyn PreKeyStore>,
    sessions: Box<dyn SessionStore>,
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
            .map_err(|e| TransportError::SignalProtocol {
                reason: format!("their identity not on curve: {e}"),
            })?;

        let hs = pqxdh::pqxdh_initiator(&our_ik_x25519, bundle, &their_ik_ed).map_err(|e| {
            TransportError::SignalProtocol {
                reason: format!("PQXDH initiator: {e}"),
            }
        })?;

        // Seed the shared Double Ratchet core as initiator. `ek_secret`
        // (not the public!) seeds our ratchet secret so the responder's
        // first reply can complete the mirrored DH step.
        let okm = ratchet::expand_pqxdh_root(&hs.root_key)?;
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
            .ok_or_else(|| TransportError::SignalProtocol {
                reason: "signed prekey not found".into(),
            })?;
        let our_spk_secret = StaticSecret::from(to_32(&spk_data, "signed prekey")?);

        let our_opk_secret = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self.prekeys.load_prekey(otpk_id)?.ok_or_else(|| {
                TransportError::SignalProtocol {
                    reason: "one-time prekey not found".into(),
                }
            })?;
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
            .ok_or_else(|| TransportError::SignalProtocol {
                reason: format!("ML-KEM secret not found for ({pq_id}, {pq_kind:?})"),
            })?;
        let our_ml_kem_secret =
            MlKemSecret::from_secret_bytes(&pq_secret_bytes).ok_or_else(|| {
                TransportError::SignalProtocol {
                    reason: "ML-KEM secret wrong length".into(),
                }
            })?;

        let initiator_ik_ed =
            VerifyingKey::from_bytes(&to_32(their_identity_key, "their identity")?).map_err(
                |e| TransportError::SignalProtocol {
                    reason: format!("their identity not on curve: {e}"),
                },
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
        .map_err(|e| TransportError::SignalProtocol {
            reason: format!("PQXDH responder: {e}"),
        })?;

        if pq_kind == PqKeyKind::OneTime {
            self.prekeys.remove_pq_secret(pq_id, PqKeyKind::OneTime)?;
        }
        if let Some(otpk_id) = one_time_prekey_id {
            self.prekeys.remove_prekey(otpk_id)?;
        }

        // Seed the shared Double Ratchet core as responder (chain
        // assignment mirrors the initiator's). The SPK SECRET seeds our
        // ratchet secret — the initiator's first message DHs against it.
        let okm = ratchet::expand_pqxdh_root(&root_key_z)?;
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
            TransportError::SignalSessionNotFound {
                peer: peer_address.to_string(),
            }
        })?;
        let mut session = RatchetState::deserialize(&session_data)?;
        let output = session.encrypt_step(plaintext)?;
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
            TransportError::SignalSessionNotFound {
                peer: peer_address.to_string(),
            }
        })?;
        let mut session = RatchetState::deserialize(&session_data)?;
        let plaintext = session.decrypt_step(message)?;
        self.sessions
            .store_session(peer_address, &session.serialize())?;
        Ok(plaintext)
    }

    /// Check if a session exists with a peer.
    pub fn has_session(&self, peer_address: &str) -> Result<bool> {
        Ok(self.sessions.has_session(peer_address)?)
    }

    /// Delete session with a peer.
    pub fn delete_session(&self, peer_address: &str) -> Result<()> {
        Ok(self.sessions.delete_session(peer_address)?)
    }

    /// Load a signed prekey's private key bytes from the store.
    ///
    /// Used by the identity ceremony to extract prekey material for
    /// persistence to the OS keyring.
    pub fn load_signed_prekey(&self, id: u32) -> Result<Vec<u8>> {
        self.prekeys
            .load_signed_prekey(id)?
            .ok_or_else(|| TransportError::SignalProtocol {
                reason: format!("signed prekey {id} not found"),
            })
    }

    /// Load a one-time prekey's private key bytes from the store.
    ///
    /// Used by the identity ceremony to extract prekey material for
    /// persistence to the OS keyring.
    pub fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        Ok(self.prekeys.load_prekey(id)?)
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

#[cfg(test)]
#[path = "signal_session/tests.rs"]
mod tests;
