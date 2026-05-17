//! HKDF key derivation from the Noise session handshake hash.
//!
//! Derives TWO directional keys for the bulk data plane:
//! - `initiator_to_responder`: client encrypts, server decrypts
//! - `responder_to_initiator`: server encrypts, client decrypts
//!
//! This prevents AES-GCM nonce reuse: both sides start their nonce
//! counters at 0, but encrypt with different keys. A single symmetric
//! key with independent nonce counters is CATASTROPHIC — the same
//! (key, nonce) pair encrypts different plaintext on each side.
//!
//! Domain-separated labels: GCM and AEGIS derive independent key pairs
//! from the same handshake hash — no key relationship.

use aws_lc_rs::hkdf;
use zeroize::Zeroize;

use super::cipher::BulkCipher;

const BULK_SALT: &[u8] = b"rekindle/bulk/v1/salt";

const BULK_I2R_LABEL_GCM: &[u8] = b"rekindle/bulk/v1/initiator-to-responder/aes-256-gcm";
const BULK_R2I_LABEL_GCM: &[u8] = b"rekindle/bulk/v1/responder-to-initiator/aes-256-gcm";
const BULK_I2R_LABEL_AEGIS: &[u8] = b"rekindle/bulk/v1/initiator-to-responder/aegis-128l";
const BULK_R2I_LABEL_AEGIS: &[u8] = b"rekindle/bulk/v1/responder-to-initiator/aegis-128l";

struct HkdfLen32;
impl hkdf::KeyType for HkdfLen32 {
    fn len(&self) -> usize {
        32
    }
}

/// A pair of directional bulk ciphers derived from the handshake hash.
pub struct BulkKeyPair {
    /// Key for initiator (client) → responder (server) direction.
    pub initiator_send: BulkCipher,
    /// Key for responder (server) → initiator (client) direction.
    pub responder_send: BulkCipher,
}

fn derive_one_key(handshake_hash: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, BULK_SALT);
    let prk = salt.extract(handshake_hash);
    let info = [label];
    let okm = prk
        .expand(&info, HkdfLen32)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail");
    let mut key = [0u8; 32];
    okm.fill(&mut key).expect("fill 32 bytes from 32-byte OKM");
    key
}

/// Derive directional bulk key pair (AES-256-GCM default).
pub fn derive_bulk_key_pair(handshake_hash: &[u8; 32]) -> BulkKeyPair {
    derive_bulk_key_pair_for_algorithm(handshake_hash, rekindle_aead::AeadAlgorithm::Aes256Gcm)
}

/// Derive directional bulk key pair for a specific algorithm.
pub fn derive_bulk_key_pair_for_algorithm(
    handshake_hash: &[u8; 32],
    algorithm: rekindle_aead::AeadAlgorithm,
) -> BulkKeyPair {
    let (i2r_label, r2i_label) = match algorithm {
        rekindle_aead::AeadAlgorithm::Aes256Gcm => (BULK_I2R_LABEL_GCM, BULK_R2I_LABEL_GCM),
        rekindle_aead::AeadAlgorithm::Aegis128L => (BULK_I2R_LABEL_AEGIS, BULK_R2I_LABEL_AEGIS),
    };

    let mut key_i2r = derive_one_key(handshake_hash, i2r_label);
    let mut key_r2i = derive_one_key(handshake_hash, r2i_label);

    let initiator_send = BulkCipher::new(&key_i2r);
    let responder_send = BulkCipher::new(&key_r2i);

    key_i2r.zeroize();
    key_r2i.zeroize();

    BulkKeyPair {
        initiator_send,
        responder_send,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let h = [0x42u8; 32];
        let p1 = derive_bulk_key_pair(&h);
        let p2 = derive_bulk_key_pair(&h);
        // Same hash → same keys (verify by encrypting same data)
        let mut buf1 = b"test".to_vec();
        let mut buf2 = b"test".to_vec();
        let tag1 = p1.initiator_send.seal_in_place(0, b"", &mut buf1).unwrap();
        let tag2 = p2.initiator_send.seal_in_place(0, b"", &mut buf2).unwrap();
        assert_eq!(buf1, buf2);
        assert_eq!(tag1, tag2);
    }

    #[test]
    fn directional_keys_differ() {
        let h = [0x42u8; 32];
        let pair = derive_bulk_key_pair(&h);
        // Encrypt same plaintext with same nonce on both ciphers
        let mut buf_i2r = b"hello".to_vec();
        let mut buf_r2i = b"hello".to_vec();
        let tag_i2r = pair.initiator_send.seal_in_place(0, b"", &mut buf_i2r).unwrap();
        let tag_r2i = pair.responder_send.seal_in_place(0, b"", &mut buf_r2i).unwrap();
        // Different keys → different ciphertext even with same nonce
        assert_ne!(buf_i2r, buf_r2i);
        assert_ne!(tag_i2r, tag_r2i);
    }

    #[test]
    fn cross_direction_decrypt_works() {
        let h = [0x42u8; 32];
        let pair = derive_bulk_key_pair(&h);
        // Encrypt with initiator_send, decrypt with a fresh pair's initiator_send (same key)
        let pair2 = derive_bulk_key_pair(&h);
        let mut buf = b"roundtrip".to_vec();
        let tag = pair.initiator_send.seal_in_place(0, b"aad", &mut buf).unwrap();
        let mut ct = Vec::with_capacity(buf.len() + 16);
        ct.extend_from_slice(&buf);
        ct.extend_from_slice(&tag);
        let pt_len = pair2.initiator_send.open_in_place(0, b"aad", &mut ct).unwrap();
        assert_eq!(&ct[..pt_len], b"roundtrip");
    }

    #[test]
    fn cross_direction_decrypt_fails_with_wrong_key() {
        let h = [0x42u8; 32];
        let pair = derive_bulk_key_pair(&h);
        let mut buf = b"secret".to_vec();
        let tag = pair.initiator_send.seal_in_place(0, b"", &mut buf).unwrap();
        let mut ct = Vec::with_capacity(buf.len() + 16);
        ct.extend_from_slice(&buf);
        ct.extend_from_slice(&tag);
        // Try to decrypt with responder_send (wrong direction) — must fail
        assert!(pair.responder_send.open_in_place(0, b"", &mut ct).is_err());
    }

    #[test]
    fn different_hashes_different_keys() {
        let p1 = derive_bulk_key_pair(&[0x42; 32]);
        let p2 = derive_bulk_key_pair(&[0x43; 32]);
        let mut buf1 = b"x".to_vec();
        let mut buf2 = b"x".to_vec();
        let tag1 = p1.initiator_send.seal_in_place(0, b"", &mut buf1).unwrap();
        let tag2 = p2.initiator_send.seal_in_place(0, b"", &mut buf2).unwrap();
        assert_ne!(buf1, buf2);
        assert_ne!(tag1, tag2);
    }
}
