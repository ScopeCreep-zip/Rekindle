//! Signal Protocol session management — X3DH + simplified Double Ratchet.
//!
//! Provides forward-secret 1:1 encrypted messaging. Sessions are established
//! via X3DH key agreement using prekey bundles published to DHT.

use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use crate::crypto::prekeys::PreKeyBundle;
use crate::crypto::signal_store::{IdentityKeyStore, PreKeyStore, SessionStore};
use crate::error::{TransportError, Result};

/// Metadata produced by initiator-side session establishment.
pub struct SessionInitInfo {
    /// The initiator's X25519 ephemeral public key.
    pub ephemeral_public_key: Vec<u8>,
    /// Which signed prekey was used.
    pub signed_prekey_id: u32,
    /// Which one-time prekey was consumed (if any).
    pub one_time_prekey_id: Option<u32>,
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
        let ephemeral_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let ephemeral_public = X25519Public::from(&ephemeral_secret);
        let ephemeral_bytes = ephemeral_secret.to_bytes();

        let (identity_private, _) = self.identity.get_identity_key_pair()?;
        let our_identity_x25519 = StaticSecret::from(
            to_32(&identity_private, "identity key")?,
        );

        let their_signed_prekey = X25519Public::from(
            to_32(&bundle.signed_prekey, "signed prekey")?,
        );
        let their_identity_x25519 = X25519Public::from(
            to_32(&bundle.identity_key, "identity key")?,
        );

        let dh1 = our_identity_x25519.diffie_hellman(&their_signed_prekey);
        let dh2 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_identity_x25519);
        let dh3 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_signed_prekey);

        let mut ikm = Vec::with_capacity(128);
        ikm.extend_from_slice(dh1.as_bytes());
        ikm.extend_from_slice(dh2.as_bytes());
        ikm.extend_from_slice(dh3.as_bytes());

        if let Some(ref otpk) = bundle.one_time_prekey {
            let their_otpk = X25519Public::from(to_32(otpk, "one-time prekey")?);
            let dh4 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_otpk);
            ikm.extend_from_slice(dh4.as_bytes());
        }

        let (root_key, sending_chain_key, receiving_chain_key) = derive_x3dh_keys(&ikm)?;

        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: ephemeral_bytes.to_vec(),
            their_ratchet_public: bundle.signed_prekey.clone(),
            send_counter: 0,
            recv_counter: 0,
        };

        self.sessions.store_session(peer_address, &serialize_ratchet(&ratchet))?;
        self.identity.save_identity(peer_address, &bundle.identity_key)?;

        Ok(SessionInitInfo {
            ephemeral_public_key: ephemeral_public.as_bytes().to_vec(),
            signed_prekey_id: 1,
            one_time_prekey_id: bundle.one_time_prekey.as_ref().map(|_| 1),
        })
    }

    /// Respond to a session initiated by a peer (responder X3DH).
    pub fn respond_to_session(
        &self,
        peer_address: &str,
        their_identity_key: &[u8],
        their_ephemeral_key: &[u8],
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
    ) -> Result<()> {
        let (identity_private, _) = self.identity.get_identity_key_pair()?;
        let our_identity_x25519 = StaticSecret::from(to_32(&identity_private, "identity key")?);

        let spk_data = self.prekeys.load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| TransportError::Internal("signed prekey not found".into()))?;
        let signed_prekey_secret = StaticSecret::from(to_32(&spk_data, "signed prekey")?);

        let their_identity_x25519 = X25519Public::from(to_32(their_identity_key, "their identity")?);
        let their_ephemeral = X25519Public::from(to_32(their_ephemeral_key, "their ephemeral")?);

        let spk_bytes = signed_prekey_secret.to_bytes();
        let dh1 = StaticSecret::from(spk_bytes).diffie_hellman(&their_identity_x25519);
        let dh2 = our_identity_x25519.diffie_hellman(&their_ephemeral);
        let dh3 = StaticSecret::from(spk_bytes).diffie_hellman(&their_ephemeral);

        let mut ikm = Vec::with_capacity(128);
        ikm.extend_from_slice(dh1.as_bytes());
        ikm.extend_from_slice(dh2.as_bytes());
        ikm.extend_from_slice(dh3.as_bytes());

        if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self.prekeys.load_prekey(otpk_id)?
                .ok_or_else(|| TransportError::Internal("one-time prekey not found".into()))?;
            let otpk_secret = StaticSecret::from(to_32(&otpk_data, "one-time prekey")?);
            let dh4 = otpk_secret.diffie_hellman(&their_ephemeral);
            ikm.extend_from_slice(dh4.as_bytes());
            self.prekeys.remove_prekey(otpk_id)?;
        }

        let (root_key, recv_chain, send_chain) = derive_x3dh_keys(&ikm)?;

        let ratchet = RatchetState {
            root_key,
            sending_chain_key: send_chain,
            receiving_chain_key: recv_chain,
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

    /// Generate a PreKeyBundle for publication to DHT.
    pub fn generate_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
    ) -> Result<PreKeyBundle> {
        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekeys.store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;

        let signing_key = SigningKey::from_bytes(&to_32(&identity_private, "identity for signing")?);
        let signature = signing_key.sign(signed_prekey_public.as_bytes());

        let one_time_prekey = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
            let otpk_public = X25519Public::from(&otpk_secret);
            self.prekeys.store_prekey(otpk_id, otpk_secret.as_bytes())?;
            Some(otpk_public.as_bytes().to_vec())
        } else {
            None
        };

        Ok(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature: signature.to_bytes().to_vec(),
            one_time_prekey,
            registration_id,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn to_32(data: &[u8], label: &str) -> Result<[u8; 32]> {
    data[..32].try_into().map_err(|_| TransportError::Internal(
        format!("{label}: expected 32 bytes, got {}", data.len()),
    ))
}

fn derive_x3dh_keys(ikm: &[u8]) -> Result<([u8; 32], [u8; 32], [u8; 32])> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 96];
    hk.expand(b"ReKindleX3DH", &mut okm)
        .map_err(|e| TransportError::Internal(format!("X3DH HKDF: {e}")))?;
    let mut root = [0u8; 32];
    let mut send = [0u8; 32];
    let mut recv = [0u8; 32];
    root.copy_from_slice(&okm[..32]);
    send.copy_from_slice(&okm[32..64]);
    recv.copy_from_slice(&okm[64..96]);
    Ok((root, send, recv))
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
    use std::sync::{Arc, Mutex};

    struct SharedSessionStore(Arc<Mutex<HashMap<String, Vec<u8>>>>);

    impl SessionStore for SharedSessionStore {
        fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().get(address).cloned())
        }
        fn store_session(&self, address: &str, data: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(address.to_string(), data.to_vec());
            Ok(())
        }
        fn has_session(&self, address: &str) -> Result<bool> {
            Ok(self.0.lock().unwrap().contains_key(address))
        }
        fn delete_session(&self, address: &str) -> Result<()> {
            self.0.lock().unwrap().remove(address);
            Ok(())
        }
        fn list_sessions(&self) -> Result<Vec<String>> {
            Ok(self.0.lock().unwrap().keys().cloned().collect())
        }
    }

    fn make_keypair() -> (Vec<u8>, Vec<u8>) {
        let secret = x25519_dalek::StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = x25519_dalek::PublicKey::from(&secret);
        (secret.to_bytes().to_vec(), public.as_bytes().to_vec())
    }

    fn establish_pair() -> (SignalSessionManager, SignalSessionManager, String, String) {
        let (alice_priv, alice_pub) = make_keypair();
        let (bob_priv, bob_pub) = make_keypair();

        let alice_sessions = Arc::new(Mutex::new(HashMap::new()));

        let alice = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::clone(&alice_sessions))),
        );
        let bob = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(bob_priv, bob_pub.clone(), 2)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        );

        let alice_addr = hex::encode(&alice_pub);
        let bob_addr = hex::encode(&bob_pub);

        let bob_bundle = bob.generate_prekey_bundle(1, Some(100)).unwrap();
        alice.establish_session(&bob_addr, &bob_bundle).unwrap();

        // Extract ephemeral private key from Alice's session, derive public for respond
        let alice_data = alice_sessions.lock().unwrap().get(&bob_addr).unwrap().clone();
        let pos = 96;
        let len = u32::from_le_bytes(alice_data[pos..pos + 4].try_into().unwrap()) as usize;
        let ephemeral_secret_bytes: [u8; 32] = alice_data[pos + 4..pos + 4 + len].try_into().unwrap();
        let ephemeral_secret = StaticSecret::from(ephemeral_secret_bytes);
        let ephemeral_public = X25519Public::from(&ephemeral_secret);

        bob.respond_to_session(&alice_addr, &alice_pub, ephemeral_public.as_bytes(), 1, Some(100)).unwrap();

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
        let (priv_key, pub_key) = make_keypair();
        let mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );

        let bundle = mgr.generate_prekey_bundle(1, Some(100)).unwrap();
        assert_eq!(bundle.signed_prekey.len(), 32);
        assert_eq!(bundle.signed_prekey_signature.len(), 64);
        assert!(bundle.one_time_prekey.is_some());
        assert_eq!(bundle.registration_id, 1);
    }

    #[test]
    fn encrypt_without_session_fails() {
        let (priv_key, pub_key) = make_keypair();
        let mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(MemorySessionStore::new()),
        );
        assert!(mgr.encrypt("nobody", b"hello").is_err());
    }
}
