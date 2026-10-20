use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;
use rekindle_types::domains;

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Symmetric key for encrypting DHT record contents.
///
/// Derived by HKDF from the Ed25519 secret (the account key), so only the
/// owner can read.
///
/// Encryption uses XChaCha20-Poly1305 (24-byte nonce, 16-byte tag).
#[derive(ZeroizeOnDrop)]
pub struct DhtRecordKey {
    key: [u8; 32],
}

impl DhtRecordKey {
    /// Derive the account encryption key from an Ed25519 secret key.
    ///
    /// Uses HKDF-SHA256 with no salt and info = `b"rekindle-account-v1"`.
    pub fn derive_account_key(ed25519_secret: &[u8; 32]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, ed25519_secret);
        let mut key = [0u8; 32];
        hk.expand(domains::ACCOUNT_KEY.as_bytes(), &mut key)
            .expect("32-byte output is valid for HKDF-SHA256");
        Self { key }
    }

    /// Encrypt plaintext with XChaCha20-Poly1305.
    ///
    /// Returns `[24-byte nonce || ciphertext || 16-byte tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
        use chacha20poly1305::XChaCha20Poly1305;
        use chacha20poly1305::XNonce;

        let cipher = XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|e| CryptoError::encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        chacha20poly1305::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::encryption(e.to_string()))?;

        let mut output = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext produced by [`Self::encrypt`].
    ///
    /// Expects `[24-byte nonce || ciphertext || 16-byte tag]`.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::XChaCha20Poly1305;
        use chacha20poly1305::XNonce;

        if data.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::decryption("ciphertext too short".to_string()));
        }

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);

        let cipher = XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|e| CryptoError::decryption(e.to_string()))?;

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::decryption(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_account_key_derivation() {
        let secret = [42u8; 32];
        let key1 = DhtRecordKey::derive_account_key(&secret);
        let key2 = DhtRecordKey::derive_account_key(&secret);
        assert_eq!(key1.key, key2.key);

        // Different secret → different key
        let other = [43u8; 32];
        let key3 = DhtRecordKey::derive_account_key(&other);
        assert_ne!(key1.key, key3.key);
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = DhtRecordKey::derive_account_key(&[7u8; 32]);
        let plaintext = b"hello rekindle DHT";

        let ciphertext = key.encrypt(plaintext).unwrap();
        assert_ne!(&ciphertext[NONCE_LEN..], plaintext);

        let decrypted = key.decrypt(&ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_rejection() {
        let key1 = DhtRecordKey::derive_account_key(&[1u8; 32]);
        let key2 = DhtRecordKey::derive_account_key(&[2u8; 32]);

        let ciphertext = key1.encrypt(b"secret data").unwrap();
        assert!(key2.decrypt(&ciphertext).is_err());
    }

    #[test]
    fn short_ciphertext_rejected() {
        let key = DhtRecordKey::derive_account_key(&[1u8; 32]);
        assert!(key.decrypt(&[0u8; 10]).is_err());
    }
}
