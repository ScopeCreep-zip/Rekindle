//! Signal Protocol session management — PQXDH + simplified Double Ratchet.
//!
//! Phase 3b of the decomposed-harvest plan replaced classical X3DH with
//! PQXDH for the daemon-track Signal subsystem. Shares the same PQXDH
//! handshake primitives as `rekindle-crypto::signal::pqxdh` via a direct
//! crate dependency.

use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rekindle_crypto::signal::pqxdh::{
    self, verify::pq_signing_payload, verify::spk_signing_payload,
};
use rekindle_secrets::pq_keys::MlKemSecret;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use crate::crypto::prekeys::PreKeyBundle;
use crate::crypto::signal_store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use crate::error::{TransportError, Result};

/// Fixed identifier for the per-identity ML-KEM-768 last-resort key.
pub const PQ_LR_ID: u32 = 0;

/// Metadata produced by initiator-side session establishment.
pub struct SessionInitInfo {
    /// The initiator's X25519 ephemeral public key.
    pub ephemeral_public_key: Vec<u8>,
    /// Which signed prekey was used.
    pub signed_prekey_id: u32,
    /// Which one-time prekey was consumed (if any).
    pub one_time_prekey_id: Option<u32>,
    /// PQXDH ML-KEM-768 ciphertext (1088 bytes).
    pub ml_kem_ciphertext: Vec<u8>,
    /// Which one-time PQ prekey was consumed (None = LastResort at PQ_LR_ID).
    pub used_ot_pqpk_id: Option<u32>,
}

/// Manages Signal Protocol sessions for 1:1 encrypted messaging.
pub struct SignalSessionManager {
    identity: Box<dyn IdentityKeyStore>,
    prekeys: Box<dyn PreKeyStore>,
    sessions: Box<dyn SessionStore>,
}

/// Internal ratchet state.
#[derive(Clone)]
struct RatchetState {
    root_key: [u8; 32],
    sending_chain_key: [u8; 32],
    receiving_chain_key: [u8; 32],
    our_ratchet_secret: Vec<u8>,
    their_ratchet_public: Vec<u8>,
    send_counter: u64,
    recv_counter: u64,
}

impl SignalSessionManager {
    pub fn new(
        identity: Box<dyn IdentityKeyStore>,
        prekeys: Box<dyn PreKeyStore>,
        sessions: Box<dyn SessionStore>,
    ) -> Self {
        Self { identity, prekeys, sessions }
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

        // Expand root_key into chain keys (mirror of rekindle-crypto).
        let hk = Hkdf::<Sha256>::new(None, &*hs.root_key);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindlePQXDH", &mut okm)
            .map_err(|e| TransportError::Internal(format!("HKDF expand: {e}")))?;
        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        sending_chain_key.copy_from_slice(&okm[32..64]);
        receiving_chain_key.copy_from_slice(&okm[64..96]);

        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: hs.ek_public.to_vec(),
            their_ratchet_public: bundle.signed_prekey.clone(),
            send_counter: 0,
            recv_counter: 0,
        };

        self.sessions.store_session(peer_address, &serialize_ratchet(&ratchet))?;
        self.identity.save_identity(peer_address, &bundle.identity_key)?;

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

        let spk_data = self.prekeys.load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| TransportError::Internal("signed prekey not found".into()))?;
        let our_spk_secret = StaticSecret::from(to_32(&spk_data, "signed prekey")?);

        let our_opk_secret = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self.prekeys.load_prekey(otpk_id)?
                .ok_or_else(|| TransportError::Internal("one-time prekey not found".into()))?;
            Some(StaticSecret::from(to_32(&otpk_data, "one-time prekey")?))
        } else {
            None
        };

        let (pq_kind, pq_id) = match used_ot_pqpk_id {
            Some(id) => (PqKeyKind::OneTime, id),
            None => (PqKeyKind::LastResort, PQ_LR_ID),
        };
        let pq_secret_bytes = self.prekeys.load_pq_secret(pq_id, pq_kind)?
            .ok_or_else(|| TransportError::Internal(format!(
                "ML-KEM secret not found for ({pq_id}, {pq_kind:?})"
            )))?;
        let our_ml_kem_secret = MlKemSecret::from_secret_bytes(&pq_secret_bytes)
            .ok_or_else(|| TransportError::Internal("ML-KEM secret wrong length".into()))?;

        let initiator_ik_ed = VerifyingKey::from_bytes(&to_32(their_identity_key, "their identity")?)
            .map_err(|e| TransportError::Internal(format!("their identity not on curve: {e}")))?;

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

        let hk = Hkdf::<Sha256>::new(None, &*root_key_z);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindlePQXDH", &mut okm)
            .map_err(|e| TransportError::Internal(format!("HKDF expand: {e}")))?;
        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        receiving_chain_key.copy_from_slice(&okm[32..64]);
        sending_chain_key.copy_from_slice(&okm[64..96]);

        let spk_bytes = our_spk_secret.to_bytes();
        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: spk_bytes.to_vec(),
            their_ratchet_public: their_ephemeral_key.to_vec(),
            send_counter: 0,
            recv_counter: 0,
        };

        self.sessions.store_session(peer_address, &serialize_ratchet(&ratchet))?;
        self.identity.save_identity(peer_address, their_identity_key)?;
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
        let session_data = self.sessions.load_session(peer_address)?
            .ok_or_else(|| TransportError::Internal(format!("no Signal session for {peer_address}")))?;

        let mut ratchet = deserialize_ratchet(&session_data)?;

        // DH ratchet step: new ephemeral → DH with their ratchet public → new root + sending chain
        let new_ratchet_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let new_ratchet_public = X25519Public::from(&new_ratchet_secret);

        let their_ratchet = X25519Public::from(
            to_32(&ratchet.their_ratchet_public, "their ratchet public")?,
        );
        let dh_output = new_ratchet_secret.diffie_hellman(&their_ratchet);

        // Derive new root key and sending chain key from DH output + old root key
        let mut ratchet_ikm = Vec::with_capacity(64);
        ratchet_ikm.extend_from_slice(&ratchet.root_key);
        ratchet_ikm.extend_from_slice(dh_output.as_bytes());
        let hk_ratchet = Hkdf::<Sha256>::new(None, &ratchet_ikm);
        let mut new_root = [0u8; 32];
        let mut new_sending_chain = [0u8; 32];
        hkdf_expand(&hk_ratchet, b"ReKindleRootKey", &mut new_root)?;
        hkdf_expand(&hk_ratchet, b"ReKindleChainRatchet", &mut new_sending_chain)?;

        ratchet.root_key = new_root;
        ratchet.sending_chain_key = new_sending_chain;
        // Store the private key so decrypt can DH with the peer's next ratchet public
        ratchet.our_ratchet_secret = new_ratchet_secret.to_bytes().to_vec();

        // Derive message key from the new sending chain key
        let hk = Hkdf::<Sha256>::new(None, &ratchet.sending_chain_key);
        let mut message_key = [0u8; 32];
        let mut next_chain_key = [0u8; 32];
        hkdf_expand(&hk, b"ReKindleMsgKey", &mut message_key)?;
        hkdf_expand(&hk, b"ReKindleChainKey", &mut next_chain_key)?;

        ratchet.sending_chain_key = next_chain_key;
        ratchet.send_counter += 1;

        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| TransportError::Internal(format!("AES init: {e}")))?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&ratchet.send_counter.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher.encrypt(nonce, plaintext)
            .map_err(|e| TransportError::Internal(format!("AES encrypt: {e}")))?;

        // Wire format: ratchet_public(32) || counter(8) || nonce(12) || ciphertext
        let mut output = Vec::with_capacity(32 + 8 + 12 + ciphertext.len());
        output.extend_from_slice(new_ratchet_public.as_bytes());
        output.extend_from_slice(&ratchet.send_counter.to_le_bytes());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        self.sessions.store_session(peer_address, &serialize_ratchet(&ratchet))?;
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
        // 32 ratchet_public + 8 counter + 12 nonce + at least 16 tag = 68 minimum
        if message.len() < 52 {
            return Err(TransportError::Internal("Signal message too short".into()));
        }

        let session_data = self.sessions.load_session(peer_address)?
            .ok_or_else(|| TransportError::Internal(format!("no Signal session for {peer_address}")))?;

        let mut ratchet = deserialize_ratchet(&session_data)?;

        // Extract sender's new ratchet public key
        let their_new_ratchet_pub = to_32(&message[..32], "sender ratchet public")?;
        let nonce_bytes: [u8; 12] = message[40..52].try_into()
            .map_err(|_| TransportError::Internal("invalid nonce".into()))?;
        let ciphertext = &message[52..];

        // DH ratchet step: DH(our_ratchet_secret, their_new_ratchet_public) → new root + receiving chain
        let our_ratchet_secret = StaticSecret::from(
            to_32(&ratchet.our_ratchet_secret, "our ratchet secret")?,
        );
        let their_ratchet = X25519Public::from(their_new_ratchet_pub);
        let dh_output = our_ratchet_secret.diffie_hellman(&their_ratchet);

        let mut ratchet_ikm = Vec::with_capacity(64);
        ratchet_ikm.extend_from_slice(&ratchet.root_key);
        ratchet_ikm.extend_from_slice(dh_output.as_bytes());
        let hk_ratchet = Hkdf::<Sha256>::new(None, &ratchet_ikm);
        let mut new_root = [0u8; 32];
        let mut new_receiving_chain = [0u8; 32];
        hkdf_expand(&hk_ratchet, b"ReKindleRootKey", &mut new_root)?;
        hkdf_expand(&hk_ratchet, b"ReKindleChainRatchet", &mut new_receiving_chain)?;

        ratchet.root_key = new_root;
        ratchet.receiving_chain_key = new_receiving_chain;
        ratchet.their_ratchet_public = their_new_ratchet_pub.to_vec();

        // Derive message key from the new receiving chain key
        let hk = Hkdf::<Sha256>::new(None, &ratchet.receiving_chain_key);
        let mut message_key = [0u8; 32];
        let mut next_chain_key = [0u8; 32];
        hkdf_expand(&hk, b"ReKindleMsgKey", &mut message_key)?;
        hkdf_expand(&hk, b"ReKindleChainKey", &mut next_chain_key)?;

        ratchet.receiving_chain_key = next_chain_key;
        ratchet.recv_counter += 1;

        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| TransportError::Internal(format!("AES init: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|e| TransportError::Internal(format!("AES decrypt: {e}")))?;

        self.sessions.store_session(peer_address, &serialize_ratchet(&ratchet))?;
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
        let signing_key = SigningKey::from_bytes(&to_32(&identity_private, "identity for signing")?);

        // X25519 signed prekey.
        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekeys.store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;
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
            self.prekeys.store_pq_secret(id, PqKeyKind::OneTime, ot_secret.as_secret_bytes())?;
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
    data[..32].try_into().map_err(|_| TransportError::Internal(
        format!("{label}: expected 32 bytes, got {}", data.len()),
    ))
}

fn hkdf_expand(hk: &Hkdf<Sha256>, info: &[u8], out: &mut [u8; 32]) -> Result<()> {
    hk.expand(info, out)
        .map_err(|e| TransportError::Internal(format!("HKDF expand: {e}")))
}

fn serialize_ratchet(state: &RatchetState) -> Vec<u8> {
    let mut data = Vec::with_capacity(128);
    data.extend_from_slice(&state.root_key);
    data.extend_from_slice(&state.sending_chain_key);
    data.extend_from_slice(&state.receiving_chain_key);
    #[allow(clippy::cast_possible_truncation)]
    let our_len = state.our_ratchet_secret.len() as u32;
    data.extend_from_slice(&our_len.to_le_bytes());
    data.extend_from_slice(&state.our_ratchet_secret);
    #[allow(clippy::cast_possible_truncation)]
    let their_len = state.their_ratchet_public.len() as u32;
    data.extend_from_slice(&their_len.to_le_bytes());
    data.extend_from_slice(&state.their_ratchet_public);
    data.extend_from_slice(&state.send_counter.to_le_bytes());
    data.extend_from_slice(&state.recv_counter.to_le_bytes());
    data
}

fn deserialize_ratchet(data: &[u8]) -> Result<RatchetState> {
    if data.len() < 112 {
        return Err(TransportError::Internal("invalid Signal session data".into()));
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

    let our_len = u32::from_le_bytes(
        data[pos..pos + 4].try_into()
            .map_err(|_| TransportError::Internal("corrupt session".into()))?,
    ) as usize;
    pos += 4;
    let our_ratchet_secret = data[pos..pos + our_len].to_vec();
    pos += our_len;

    let their_len = u32::from_le_bytes(
        data[pos..pos + 4].try_into()
            .map_err(|_| TransportError::Internal("corrupt session".into()))?,
    ) as usize;
    pos += 4;
    let their_ratchet_public = data[pos..pos + their_len].to_vec();
    pos += their_len;

    let send_counter = u64::from_le_bytes(
        data[pos..pos + 8].try_into()
            .map_err(|_| TransportError::Internal("corrupt session".into()))?,
    );
    pos += 8;

    let recv_counter = u64::from_le_bytes(
        data[pos..pos + 8].try_into()
            .map_err(|_| TransportError::Internal("corrupt session".into()))?,
    );

    Ok(RatchetState {
        root_key, sending_chain_key, receiving_chain_key,
        our_ratchet_secret, their_ratchet_public,
        send_counter, recv_counter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::signal_store::{MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore};
    use std::collections::HashMap;
    use std::sync::Arc;
    use parking_lot::Mutex;

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
}
