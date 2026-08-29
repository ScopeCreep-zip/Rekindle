//! THE Double Ratchet core — one stepping implementation for every track.
//!
//! Before this module existed the workspace carried TWO ratchets for the
//! same 1:1 messages: this crate's symmetric-only chain (wire header
//! `[counter(8) || nonce(12)]`, no DH healing) on the desktop track, and
//! a per-message DH ratchet (wire header `[ratchet_public(32) ||
//! counter(8) || nonce(12)]`) in `rekindle-transport` on the daemon
//! track. **They were not wire-compatible** — a DM between the two
//! tracks could not decrypt. Both `SignalSessionManager`s now step
//! through this module, so cross-track compatibility holds by
//! construction.
//!
//! The converged design is the daemon track's (the stronger of the two):
//! every `encrypt_step` generates a fresh X25519 ephemeral and mixes
//! `DH(new_ephemeral, their_ratchet_public)` into the root key, giving
//! per-message forward secrecy on the sending chain; `decrypt_step`
//! performs the mirrored DH with our stored ratchet secret.
//!
//! ## Known limitations (pre-existing, inherited from the daemon track)
//!
//! - **No skipped-message keys**: out-of-order delivery within one
//!   direction fails to decrypt (the chain has already advanced).
//! - **Crossing sends can desync**: if both peers encrypt before seeing
//!   each other's latest ratchet key, the receiver's stored ratchet
//!   secret no longer matches the DH the sender performed. Full
//!   Signal-spec header keys + skipped-key storage are future work;
//!   the DM layer's retry/re-establish path is the current backstop.
//!
//! Wire format per message:
//! `[ratchet_public(32) || counter(8 LE) || nonce(12) || ciphertext+tag]`
//! (52-byte header; minimum valid message is 68 bytes with the GCM tag).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::CryptoError;

/// Header bytes preceding the AEAD ciphertext: ratchet_public(32) +
/// counter(8) + nonce(12).
pub const HEADER_LEN: usize = 52;

/// HKDF info labels — shared by every track; changing any of these is a
/// wire break.
const LABEL_PQXDH_EXPAND: &[u8] = b"ReKindlePQXDH";
const LABEL_ROOT: &[u8] = b"ReKindleRootKey";
const LABEL_CHAIN_RATCHET: &[u8] = b"ReKindleChainRatchet";
const LABEL_MSG_KEY: &[u8] = b"ReKindleMsgKey";
const LABEL_CHAIN_KEY: &[u8] = b"ReKindleChainKey";

/// Double Ratchet session state.
///
/// Serialized via [`RatchetState::serialize`] into the session store;
/// the layout is identical on every track (it predates this module and
/// was already byte-identical between the two forks).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RatchetState {
    /// Root key — evolves with each DH ratchet step.
    root_key: [u8; 32],
    /// Sending chain key — evolves with each message sent.
    sending_chain_key: [u8; 32],
    /// Receiving chain key — evolves with each message received.
    receiving_chain_key: [u8; 32],
    /// Our current DH ratchet SECRET (X25519).
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

/// Expand a PQXDH root key into the 96-byte OKM that seeds a session:
/// `[session_root(32) || chain_a(32) || chain_b(32)]`. The initiator
/// takes `chain_a` as its sending chain; the responder mirrors.
pub fn expand_pqxdh_root(root_key: &[u8; 32]) -> Result<[u8; 96], CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, root_key);
    let mut okm = [0u8; 96];
    hk.expand(LABEL_PQXDH_EXPAND, &mut okm)
        .map_err(|e| CryptoError::SessionError(format!("HKDF expand failed: {e}")))?;
    Ok(okm)
}

impl RatchetState {
    /// Initial state for the PQXDH INITIATOR.
    ///
    /// - `okm`: [`expand_pqxdh_root`] output.
    /// - `our_ek_secret`: the initiator's ephemeral X25519 SECRET from
    ///   the handshake (`InitiatorHandshake::ek_secret`) — the responder
    ///   DHs its first reply against our `EK_A`, so this must be the
    ///   secret half, not the public. (Both pre-convergence forks stored
    ///   the PUBLIC key here; the daemon track's responder-replies-first
    ///   path could never decrypt because of it.)
    /// - `their_spk_public`: the responder's signed-prekey public from
    ///   the bundle — their initial ratchet key.
    pub fn initiator(okm: &[u8; 96], our_ek_secret: [u8; 32], their_spk_public: Vec<u8>) -> Self {
        let (root, chain_a, chain_b) = split_okm(okm);
        Self {
            root_key: root,
            sending_chain_key: chain_a,
            receiving_chain_key: chain_b,
            our_ratchet_secret: our_ek_secret.to_vec(),
            their_ratchet_public: their_spk_public,
            send_counter: 0,
            recv_counter: 0,
        }
    }

    /// Initial state for the PQXDH RESPONDER (mirrors the initiator's
    /// chain assignment).
    ///
    /// - `our_spk_secret`: our signed-prekey SECRET — the initiator's
    ///   first message DHs against `SPK_B`.
    /// - `their_ek_public`: the initiator's ephemeral public from the
    ///   handshake — their initial ratchet key.
    pub fn responder(okm: &[u8; 96], our_spk_secret: [u8; 32], their_ek_public: Vec<u8>) -> Self {
        let (root, chain_a, chain_b) = split_okm(okm);
        Self {
            root_key: root,
            sending_chain_key: chain_b,
            receiving_chain_key: chain_a,
            our_ratchet_secret: our_spk_secret.to_vec(),
            their_ratchet_public: their_ek_public,
            send_counter: 0,
            recv_counter: 0,
        }
    }

    /// Encrypt one message, advancing the ratchet.
    ///
    /// DH ratchet step: fresh ephemeral → `DH(ephemeral,
    /// their_ratchet_public)` mixed with the root key → new root +
    /// sending chain; then one symmetric chain step derives the message
    /// key. The fresh ratchet public rides the wire header so the
    /// receiver can mirror the step.
    pub fn encrypt_step(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let new_ratchet_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let new_ratchet_public = X25519Public::from(&new_ratchet_secret);

        let their_ratchet =
            X25519Public::from(to_32(&self.their_ratchet_public, "their ratchet public")?);
        let dh_output = new_ratchet_secret.diffie_hellman(&their_ratchet);

        let (new_root, new_sending_chain) = ratchet_root(&self.root_key, dh_output.as_bytes())?;
        self.root_key = new_root;
        self.sending_chain_key = new_sending_chain;
        self.our_ratchet_secret = new_ratchet_secret.to_bytes().to_vec();

        let (message_key, next_chain_key) = chain_step(&self.sending_chain_key)?;
        self.sending_chain_key = next_chain_key;
        self.send_counter += 1;

        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| CryptoError::EncryptionError(format!("AES init: {e}")))?;
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&self.send_counter.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionError(format!("AES encrypt: {e}")))?;

        let mut output = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        output.extend_from_slice(new_ratchet_public.as_bytes());
        output.extend_from_slice(&self.send_counter.to_le_bytes());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt one message, advancing the ratchet with the mirrored DH
    /// step against the sender's fresh ratchet public from the header.
    pub fn decrypt_step(&mut self, message: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if message.len() < HEADER_LEN {
            return Err(CryptoError::DecryptionError(
                "Signal message too short".into(),
            ));
        }

        let their_new_ratchet_pub = to_32(&message[..32], "sender ratchet public")?;
        let nonce_bytes: [u8; 12] = message[40..52]
            .try_into()
            .map_err(|_| CryptoError::DecryptionError("invalid nonce".into()))?;
        let ciphertext = &message[52..];

        let our_ratchet_secret =
            StaticSecret::from(to_32(&self.our_ratchet_secret, "our ratchet secret")?);
        let their_ratchet = X25519Public::from(their_new_ratchet_pub);
        let dh_output = our_ratchet_secret.diffie_hellman(&their_ratchet);

        let (new_root, new_receiving_chain) = ratchet_root(&self.root_key, dh_output.as_bytes())?;
        self.root_key = new_root;
        self.receiving_chain_key = new_receiving_chain;
        self.their_ratchet_public = their_new_ratchet_pub.to_vec();

        let (message_key, next_chain_key) = chain_step(&self.receiving_chain_key)?;
        self.receiving_chain_key = next_chain_key;
        self.recv_counter += 1;

        let cipher = Aes256Gcm::new_from_slice(&message_key)
            .map_err(|e| CryptoError::DecryptionError(format!("AES init: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionError(format!("AES decrypt: {e}")))
    }

    /// Serialize for the session store. Layout (unchanged from both
    /// pre-convergence forks, which were already byte-identical):
    /// `root(32) || send_chain(32) || recv_chain(32) ||
    ///  our_len(4 LE) || our_secret || their_len(4 LE) || their_public ||
    ///  send_counter(8 LE) || recv_counter(8 LE)`.
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(128);
        data.extend_from_slice(&self.root_key);
        data.extend_from_slice(&self.sending_chain_key);
        data.extend_from_slice(&self.receiving_chain_key);
        let our_len = u32::try_from(self.our_ratchet_secret.len())
            .expect("ratchet secret length must fit in u32");
        data.extend_from_slice(&our_len.to_le_bytes());
        data.extend_from_slice(&self.our_ratchet_secret);
        let their_len = u32::try_from(self.their_ratchet_public.len())
            .expect("ratchet public length must fit in u32");
        data.extend_from_slice(&their_len.to_le_bytes());
        data.extend_from_slice(&self.their_ratchet_public);
        data.extend_from_slice(&self.send_counter.to_le_bytes());
        data.extend_from_slice(&self.recv_counter.to_le_bytes());
        data
    }

    /// Deserialize from the session store.
    pub fn deserialize(data: &[u8]) -> Result<Self, CryptoError> {
        const CORRUPT: fn() -> CryptoError = || CryptoError::SessionError("corrupt session".into());
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
            data[pos..pos + 4].try_into().map_err(|_| CORRUPT())?,
        ))
        .map_err(|_| CORRUPT())?;
        pos += 4;
        if data.len() < pos + our_len + 4 {
            return Err(CORRUPT());
        }
        let our_ratchet_secret = data[pos..pos + our_len].to_vec();
        pos += our_len;

        let their_len = usize::try_from(u32::from_le_bytes(
            data[pos..pos + 4].try_into().map_err(|_| CORRUPT())?,
        ))
        .map_err(|_| CORRUPT())?;
        pos += 4;
        if data.len() < pos + their_len + 16 {
            return Err(CORRUPT());
        }
        let their_ratchet_public = data[pos..pos + their_len].to_vec();
        pos += their_len;

        let send_counter =
            u64::from_le_bytes(data[pos..pos + 8].try_into().map_err(|_| CORRUPT())?);
        pos += 8;
        let recv_counter =
            u64::from_le_bytes(data[pos..pos + 8].try_into().map_err(|_| CORRUPT())?);

        Ok(Self {
            root_key,
            sending_chain_key,
            receiving_chain_key,
            our_ratchet_secret,
            their_ratchet_public,
            send_counter,
            recv_counter,
        })
    }

    /// Messages sent so far on this session.
    pub fn send_counter(&self) -> u64 {
        self.send_counter
    }

    /// Messages received so far on this session.
    pub fn recv_counter(&self) -> u64 {
        self.recv_counter
    }
}

fn split_okm(okm: &[u8; 96]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let mut root = [0u8; 32];
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    root.copy_from_slice(&okm[..32]);
    a.copy_from_slice(&okm[32..64]);
    b.copy_from_slice(&okm[64..96]);
    (root, a, b)
}

/// Mix a DH output into the root key: new root + new chain key.
fn ratchet_root(root_key: &[u8; 32], dh: &[u8]) -> Result<([u8; 32], [u8; 32]), CryptoError> {
    let mut ikm = Vec::with_capacity(64);
    ikm.extend_from_slice(root_key);
    ikm.extend_from_slice(dh);
    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut new_root = [0u8; 32];
    let mut new_chain = [0u8; 32];
    hk.expand(LABEL_ROOT, &mut new_root)
        .map_err(|e| CryptoError::SessionError(format!("HKDF: {e}")))?;
    hk.expand(LABEL_CHAIN_RATCHET, &mut new_chain)
        .map_err(|e| CryptoError::SessionError(format!("HKDF: {e}")))?;
    ikm.zeroize();
    Ok((new_root, new_chain))
}

/// One symmetric chain step: message key + next chain key.
fn chain_step(chain_key: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, chain_key);
    let mut message_key = [0u8; 32];
    let mut next_chain_key = [0u8; 32];
    hk.expand(LABEL_MSG_KEY, &mut message_key)
        .map_err(|e| CryptoError::SessionError(format!("HKDF: {e}")))?;
    hk.expand(LABEL_CHAIN_KEY, &mut next_chain_key)
        .map_err(|e| CryptoError::SessionError(format!("HKDF: {e}")))?;
    Ok((message_key, next_chain_key))
}

fn to_32(bytes: &[u8], what: &str) -> Result<[u8; 32], CryptoError> {
    bytes
        .try_into()
        .map_err(|_| CryptoError::InvalidKey(format!("{what} must be 32 bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a linked initiator/responder pair the way the PQXDH
    /// handshake would: shared OKM, initiator ephemeral, responder SPK.
    fn linked_pair() -> (RatchetState, RatchetState) {
        let okm = expand_pqxdh_root(&[7u8; 32]).unwrap();
        let ek = StaticSecret::from([1u8; 32]);
        let spk = StaticSecret::from([2u8; 32]);
        let a = RatchetState::initiator(
            &okm,
            ek.to_bytes(),
            X25519Public::from(&spk).as_bytes().to_vec(),
        );
        let b = RatchetState::responder(
            &okm,
            spk.to_bytes(),
            X25519Public::from(&ek).as_bytes().to_vec(),
        );
        (a, b)
    }

    #[test]
    fn initiator_sends_first_roundtrip() {
        let (mut a, mut b) = linked_pair();
        let wire = a.encrypt_step(b"hello bob").unwrap();
        assert_eq!(b.decrypt_step(&wire).unwrap(), b"hello bob");
    }

    #[test]
    fn responder_sends_first_roundtrip() {
        // The pre-convergence forks stored the initiator's ephemeral
        // PUBLIC key as its ratchet secret, which broke exactly this
        // ordering. Guard the fix.
        let (mut a, mut b) = linked_pair();
        let wire = b.encrypt_step(b"hello alice").unwrap();
        assert_eq!(a.decrypt_step(&wire).unwrap(), b"hello alice");
    }

    #[test]
    fn ping_pong_conversation() {
        let (mut a, mut b) = linked_pair();
        for i in 0..6u8 {
            let m = [i; 24];
            if i % 2 == 0 {
                let wire = a.encrypt_step(&m).unwrap();
                assert_eq!(b.decrypt_step(&wire).unwrap(), m);
            } else {
                let wire = b.encrypt_step(&m).unwrap();
                assert_eq!(a.decrypt_step(&wire).unwrap(), m);
            }
        }
        assert_eq!(a.send_counter(), 3);
        assert_eq!(a.recv_counter(), 3);
    }

    #[test]
    fn consecutive_same_direction_messages() {
        let (mut a, mut b) = linked_pair();
        for i in 0..4u8 {
            let m = [i; 16];
            let wire = a.encrypt_step(&m).unwrap();
            assert_eq!(b.decrypt_step(&wire).unwrap(), m);
        }
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let (mut a, mut b) = linked_pair();
        let mut wire = a.encrypt_step(b"payload").unwrap();
        let last = wire.len() - 1;
        wire[last] ^= 0xFF;
        assert!(b.decrypt_step(&wire).is_err());
    }

    #[test]
    fn serialize_roundtrip_preserves_session() {
        let (mut a, mut b) = linked_pair();
        let w1 = a.encrypt_step(b"one").unwrap();
        b.decrypt_step(&w1).unwrap();

        // Persist and restore both sides mid-conversation.
        let a2 = RatchetState::deserialize(&a.serialize()).unwrap();
        let mut b2 = RatchetState::deserialize(&b.serialize()).unwrap();
        let mut a2 = a2;

        let w2 = b2.encrypt_step(b"two").unwrap();
        assert_eq!(a2.decrypt_step(&w2).unwrap(), b"two");
    }

    #[test]
    fn short_message_rejected() {
        let (_, mut b) = linked_pair();
        assert!(b.decrypt_step(&[0u8; 51]).is_err());
    }

    #[test]
    fn corrupt_session_data_rejected() {
        assert!(RatchetState::deserialize(&[0u8; 50]).is_err());
        // Length prefix pointing past the end.
        let mut data = vec![0u8; 112];
        data[96..100].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(RatchetState::deserialize(&data).is_err());
    }
}
