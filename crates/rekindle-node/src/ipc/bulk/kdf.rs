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

/// Fixed HKDF salt for bulk key derivation.
const BULK_SALT: &[u8] = b"rekindle/bulk/v1/salt";

/// HKDF info label for AES-256-GCM bulk key derivation.
const BULK_KEY_LABEL_GCM: &[u8] = b"rekindle/bulk/v1/aes-256-gcm-key";

/// HKDF info label for AEGIS-128L bulk key derivation.
/// Domain-separated from GCM to prevent key relationship: if both
/// algorithms are derived from the same handshake hash, the keys
/// are independent (different HKDF info labels).
const BULK_KEY_LABEL_AEGIS: &[u8] = b"rekindle/bulk/v1/aegis-128l-key";

/// Derive a 32-byte bulk key from the Noise handshake hash using
/// the specified algorithm's domain-separated HKDF label.
pub fn derive_bulk_key_for_algorithm(
    handshake_hash: &[u8; 32],
    algorithm: rekindle_aead::AeadAlgorithm,
) -> [u8; 32] {
    let label = match algorithm {
        rekindle_aead::AeadAlgorithm::Aes256Gcm => BULK_KEY_LABEL_GCM,
        rekindle_aead::AeadAlgorithm::Aegis128L => BULK_KEY_LABEL_AEGIS,
    };
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, BULK_SALT);
    let prk = salt.extract(handshake_hash);
    let info = [label];
    let okm = prk
        .expand(&info, HkdfLen32)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail");

    let mut key = [0u8; 32];
    okm.fill(&mut key)
        .expect("fill 32 bytes from 32-byte OKM cannot fail");
    key
}

/// Derive a 32-byte AES-256-GCM key from the Noise handshake hash.
pub fn derive_bulk_key(handshake_hash: &[u8; 32]) -> [u8; 32] {
    derive_bulk_key_for_algorithm(handshake_hash, rekindle_aead::AeadAlgorithm::Aes256Gcm)
}

/// Derive the bulk key and construct a [`BulkCipher`] with the default
/// algorithm (AES-256-GCM), zeroizing the intermediate key material.
pub fn derive_bulk_cipher(handshake_hash: &[u8; 32]) -> super::cipher::BulkCipher {
    let mut key = derive_bulk_key(handshake_hash);
    let cipher = super::cipher::BulkCipher::new(&key);
    key.zeroize();
    cipher
}

/// Derive the bulk key and construct a [`BulkCipher`] with a specific
/// algorithm, using the algorithm's domain-separated HKDF label.
pub fn derive_bulk_cipher_with_algorithm(
    handshake_hash: &[u8; 32],
    algorithm: rekindle_aead::AeadAlgorithm,
) -> Result<super::cipher::BulkCipher, super::cipher::CipherError> {
    let mut key = derive_bulk_key_for_algorithm(handshake_hash, algorithm);
    let cipher = super::cipher::BulkCipher::with_algorithm(algorithm, &key)?;
    key.zeroize();
    Ok(cipher)
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

    #[test]
    fn gcm_and_aegis_labels_produce_different_keys() {
        let h = [0x42u8; 32];
        let k_gcm = derive_bulk_key_for_algorithm(&h, rekindle_aead::AeadAlgorithm::Aes256Gcm);
        let k_aegis = derive_bulk_key_for_algorithm(&h, rekindle_aead::AeadAlgorithm::Aegis128L);
        assert_ne!(k_gcm, k_aegis, "domain-separated labels must produce different keys");
    }

    #[test]
    fn derive_bulk_key_matches_gcm_algorithm() {
        let h = [0x42u8; 32];
        let k1 = derive_bulk_key(&h);
        let k2 = derive_bulk_key_for_algorithm(&h, rekindle_aead::AeadAlgorithm::Aes256Gcm);
        assert_eq!(k1, k2);
    }
}
