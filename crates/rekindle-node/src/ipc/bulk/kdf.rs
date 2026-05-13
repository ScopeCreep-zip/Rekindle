//! HKDF key derivation from the Noise session handshake hash.
//!
//! The Noise IK handshake produces a 32-byte "handshake hash" (`h`) that
//! is the SHA-256 transcript hash binding the entire handshake. This
//! module derives the bulk AES-256-GCM key from `h` via HKDF-SHA-256.
//!
//! If a new key is needed (e.g., after suspected compromise), perform a
//! new Noise handshake to get a fresh handshake hash. Do not attempt to
//! derive multiple keys from the same hash with different parameters —
//! that requires a key rotation protocol with nonce space coordination.

use aws_lc_rs::hkdf;
use zeroize::Zeroize;

/// HKDF info label for bulk AEAD key derivation.
const BULK_KEY_LABEL: &[u8] = b"rekindle/bulk/v1/aead-key";

/// Fixed HKDF salt for bulk key derivation.
const BULK_SALT: &[u8] = b"rekindle/bulk/v1/salt";

/// Derive a 32-byte AES-256-GCM key from the Noise handshake hash.
pub fn derive_bulk_key(handshake_hash: &[u8; 32]) -> [u8; 32] {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, BULK_SALT);
    let prk = salt.extract(handshake_hash);
    let okm = prk
        .expand(&[BULK_KEY_LABEL], HkdfLen32)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail");

    let mut key = [0u8; 32];
    okm.fill(&mut key)
        .expect("fill 32 bytes from 32-byte OKM cannot fail");
    key
}

/// Derive the bulk key and construct a [`BulkCipher`], zeroizing
/// the intermediate key material.
pub fn derive_bulk_cipher(handshake_hash: &[u8; 32]) -> super::cipher::BulkCipher {
    let mut key = derive_bulk_key(handshake_hash);
    let cipher = super::cipher::BulkCipher::new(&key);
    key.zeroize();
    cipher
}

/// Newtype for HKDF output length (32 bytes = one AES-256 key).
struct HkdfLen32;

impl hkdf::KeyType for HkdfLen32 {
    fn len(&self) -> usize {
        32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_derivation() {
        let h = [0x42u8; 32];
        let k1 = derive_bulk_key(&h);
        let k2 = derive_bulk_key(&h);
        assert_eq!(k1, k2);
    }

    #[test]
    fn different_hashes_produce_different_keys() {
        let h1 = [0x42u8; 32];
        let h2 = [0x43u8; 32];
        let k1 = derive_bulk_key(&h1);
        let k2 = derive_bulk_key(&h2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_cipher_succeeds() {
        let h = [0x42u8; 32];
        let _cipher = derive_bulk_cipher(&h);
    }
}
