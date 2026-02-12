use crate::error::CryptoError;
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::store::{IdentityKeyStore, PreKeyStore, SessionStore};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use ed25519_dalek::{Signer, SigningKey};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

/// Manages Signal Protocol sessions for 1:1 encrypted messaging.
///
/// Uses X3DH for session establishment and a simplified Double Ratchet
/// for forward-secret message encryption.
#[allow(clippy::struct_field_names)] // Store suffix clarifies the role of each field
pub struct SignalSessionManager {
    identity_store: Box<dyn IdentityKeyStore>,
    prekey_store: Box<dyn PreKeyStore>,
    session_store: Box<dyn SessionStore>,
}

/// An established session's symmetric ratchet state.
#[derive(Clone)]
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
    their_ratchet_public: Vec<u8>,
    /// Send message counter.
    send_counter: u64,
    /// Receive message counter.
    recv_counter: u64,
}

impl SignalSessionManager {
    /// Create a new session manager with the given storage backends.
    pub fn new(
        identity_store: Box<dyn IdentityKeyStore>,
        prekey_store: Box<dyn PreKeyStore>,
        session_store: Box<dyn SessionStore>,
    ) -> Self {
        Self {
            identity_store,
            prekey_store,
            session_store,
        }
    }

    /// Establish a session with a peer using their `PreKeyBundle` (X3DH).
    ///
    /// This is the initiator side — called when we want to start a conversation
    /// with someone whose `PreKeyBundle` we fetched from DHT.
    pub fn establish_session(&self, peer_address: &str, bundle: &PreKeyBundle) -> Result<(), CryptoError> {
        // X3DH key agreement:
        // 1. Generate ephemeral X25519 keypair
        let ephemeral_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let ephemeral_public = X25519Public::from(&ephemeral_secret);
        let ephemeral_bytes = ephemeral_secret.to_bytes();

        // 2. Our identity X25519 key
        let (identity_private, _identity_public) = self.identity_store.get_identity_key_pair()?;
        let our_identity_x25519 = StaticSecret::from(
            <[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length".into()))?,
        );

        // 3. Their signed prekey
        let their_signed_prekey = X25519Public::from(
            <[u8; 32]>::try_from(bundle.signed_prekey.as_slice())
                .map_err(|_| CryptoError::InvalidKey("signed prekey wrong length".into()))?,
        );

        // 4. Their identity key (for X25519)
        let their_identity_x25519 = X25519Public::from(
            <[u8; 32]>::try_from(bundle.identity_key.as_slice())
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length".into()))?,
        );

        // 5. Compute shared secrets: DH1 || DH2 || DH3
        let dh1 = our_identity_x25519.diffie_hellman(&their_signed_prekey);
        let dh2 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_identity_x25519);
        let dh3 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_signed_prekey);

        // 6. Concatenate and derive root key + chain keys via HKDF
        let mut ikm = Vec::with_capacity(96);
        ikm.extend_from_slice(dh1.as_bytes());
        ikm.extend_from_slice(dh2.as_bytes());
        ikm.extend_from_slice(dh3.as_bytes());

        // Optional DH4 with one-time prekey
        if let Some(ref otpk) = bundle.one_time_prekey {
            let their_otpk = X25519Public::from(
                <[u8; 32]>::try_from(otpk.as_slice())
                    .map_err(|_| CryptoError::InvalidKey("one-time prekey wrong length".into()))?,
            );
            let dh4 = StaticSecret::from(ephemeral_bytes).diffie_hellman(&their_otpk);
            ikm.extend_from_slice(dh4.as_bytes());
        }

        let hk = Hkdf::<Sha256>::new(None, &ikm);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindleX3DH", &mut okm)
            .map_err(|e| CryptoError::SessionError(format!("HKDF expand failed: {e}")))?;

        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        sending_chain_key.copy_from_slice(&okm[32..64]);
        receiving_chain_key.copy_from_slice(&okm[64..96]);

        // 7. Serialize and store the initial ratchet state
        let ratchet = RatchetState {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret: ephemeral_public.as_bytes().to_vec(),
            their_ratchet_public: bundle.signed_prekey.clone(),
            send_counter: 0,
            recv_counter: 0,
        };

        let session_data = serialize_ratchet(&ratchet);
        self.session_store.store_session(peer_address, &session_data)?;

        // Trust their identity on first use (TOFU)
        self.identity_store.save_identity(peer_address, &bundle.identity_key)?;

        Ok(())
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
    ) -> Result<(), CryptoError> {
        // Load our identity keypair
        let (identity_private, _identity_public) = self.identity_store.get_identity_key_pair()?;
        let our_identity_x25519 = StaticSecret::from(
            <[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length".into()))?,
        );

        // Load our signed prekey private key
        let spk_data = self
            .prekey_store
            .load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| CryptoError::InvalidKey("signed prekey not found".into()))?;
        let signed_prekey_secret = StaticSecret::from(
            <[u8; 32]>::try_from(spk_data.as_slice())
                .map_err(|_| CryptoError::InvalidKey("signed prekey wrong length".into()))?,
        );

        // Their identity key as X25519 public
        let their_identity_x25519 = X25519Public::from(
            <[u8; 32]>::try_from(their_identity_key)
                .map_err(|_| CryptoError::InvalidKey("their identity key wrong length".into()))?,
        );

        // Their ephemeral key
        let their_ephemeral = X25519Public::from(
            <[u8; 32]>::try_from(their_ephemeral_key)
                .map_err(|_| CryptoError::InvalidKey("their ephemeral key wrong length".into()))?,
        );

        // Compute shared secrets (mirror of initiator):
        // DH1 = DH(our_signed_prekey, their_identity)
        // DH2 = DH(our_identity, their_ephemeral)
        // DH3 = DH(our_signed_prekey, their_ephemeral)
        let spk_bytes = signed_prekey_secret.to_bytes();
        let dh1 = StaticSecret::from(spk_bytes).diffie_hellman(&their_identity_x25519);
        let dh2 = our_identity_x25519.diffie_hellman(&their_ephemeral);
        let dh3 = StaticSecret::from(spk_bytes).diffie_hellman(&their_ephemeral);

        let mut ikm = Vec::with_capacity(96);
        ikm.extend_from_slice(dh1.as_bytes());
        ikm.extend_from_slice(dh2.as_bytes());
        ikm.extend_from_slice(dh3.as_bytes());

        // Optional DH4 with one-time prekey
        if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self
                .prekey_store
                .load_prekey(otpk_id)?
                .ok_or_else(|| CryptoError::InvalidKey("one-time prekey not found".into()))?;
            let otpk_secret = StaticSecret::from(
                <[u8; 32]>::try_from(otpk_data.as_slice())
                    .map_err(|_| CryptoError::InvalidKey("one-time prekey wrong length".into()))?,
            );
            let dh4 = otpk_secret.diffie_hellman(&their_ephemeral);
            ikm.extend_from_slice(dh4.as_bytes());

            // Consume the one-time prekey
            self.prekey_store.remove_prekey(otpk_id)?;
        }

        let hk = Hkdf::<Sha256>::new(None, &ikm);
        let mut okm = [0u8; 96];
        hk.expand(b"ReKindleX3DH", &mut okm)
            .map_err(|e| CryptoError::SessionError(format!("HKDF expand failed: {e}")))?;

        let mut root_key = [0u8; 32];
        let mut sending_chain_key = [0u8; 32];
        let mut receiving_chain_key = [0u8; 32];
        root_key.copy_from_slice(&okm[..32]);
        // Responder swaps sending/receiving relative to initiator
        receiving_chain_key.copy_from_slice(&okm[32..64]);
        sending_chain_key.copy_from_slice(&okm[64..96]);

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
        self.session_store.store_session(peer_address, &session_data)?;

        // Trust their identity on first use (TOFU)
        self.identity_store.save_identity(peer_address, their_identity_key)?;

        Ok(())
    }

    /// Encrypt a plaintext message for a peer.
    pub fn encrypt(&self, peer_address: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let session_data = self
            .session_store
            .load_session(peer_address)?
            .ok_or_else(|| CryptoError::SessionError("no session for peer".into()))?;

        let mut ratchet = deserialize_ratchet(&session_data)?;

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

        // Save updated ratchet state
        let new_session_data = serialize_ratchet(&ratchet);
        self.session_store.store_session(peer_address, &new_session_data)?;

        Ok(output)
    }

    /// Decrypt a ciphertext message from a peer.
    pub fn decrypt(&self, peer_address: &str, message: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if message.len() < 20 {
            return Err(CryptoError::DecryptionError("message too short".into()));
        }

        let session_data = self
            .session_store
            .load_session(peer_address)?
            .ok_or_else(|| CryptoError::SessionError("no session for peer".into()))?;

        let mut ratchet = deserialize_ratchet(&session_data)?;

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

        // Save updated ratchet state
        let new_session_data = serialize_ratchet(&ratchet);
        self.session_store.store_session(peer_address, &new_session_data)?;

        Ok(plaintext)
    }

    /// Check if we have an established session with a peer.
    pub fn has_session(&self, peer_address: &str) -> Result<bool, CryptoError> {
        self.session_store.has_session(peer_address)
    }

    /// Generate a `PreKeyBundle` for publication to DHT.
    ///
    /// Creates a signed prekey and optional one-time prekey, stores them
    /// in the prekey store, and returns the bundle.
    pub fn generate_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
    ) -> Result<PreKeyBundle, CryptoError> {
        let (identity_private, identity_public) = self.identity_store.get_identity_key_pair()?;
        let registration_id = self.identity_store.get_local_registration_id()?;

        // Generate signed prekey (X25519)
        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekey_store
            .store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;

        // Sign the prekey public key bytes with our Ed25519 identity key
        let signing_key = SigningKey::from_bytes(
            &<[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::InvalidKey("identity key wrong length for signing".into()))?,
        );
        let signature = signing_key.sign(signed_prekey_public.as_bytes());
        let signed_prekey_signature = signature.to_bytes().to_vec();

        // Optionally generate a one-time prekey
        let one_time_prekey = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
            let otpk_public = X25519Public::from(&otpk_secret);
            self.prekey_store
                .store_prekey(otpk_id, otpk_secret.as_bytes())?;
            Some(otpk_public.as_bytes().to_vec())
        } else {
            None
        };

        Ok(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature,
            one_time_prekey,
            registration_id,
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
    )).map_err(|_| CryptoError::SessionError("ratchet secret length overflow".into()))?;
    pos += 4;
    let our_ratchet_secret = data[pos..pos + our_len].to_vec();
    pos += our_len;

    let their_len = usize::try_from(u32::from_le_bytes(
        data[pos..pos + 4]
            .try_into()
            .map_err(|_| CryptoError::SessionError("corrupt session".into()))?,
    )).map_err(|_| CryptoError::SessionError("ratchet public length overflow".into()))?;
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
