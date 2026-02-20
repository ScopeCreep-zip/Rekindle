use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Symmetric key for encrypting DHT record contents.
///
/// Two derivation modes:
/// - **Account key**: HKDF from Ed25519 secret — only the owner can read.
/// - **Conversation key**: DH shared secret — both parties can read/write.
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
        hk.expand(b"rekindle-account-v1", &mut key)
            .expect("32-byte output is valid for HKDF-SHA256");
        Self { key }
    }

    /// Derive a conversation encryption key from a DH shared secret.
    ///
    /// Performs X25519 DH, then runs HKDF-SHA256 with info containing both
    /// public keys sorted lexicographically (so both parties derive the same key).
    pub fn derive_conversation_key(my_secret: &StaticSecret, their_public: &PublicKey) -> Self {
        let shared = my_secret.diffie_hellman(their_public);
        let my_public = PublicKey::from(my_secret);

        // Sort public keys so both parties produce the same info string
        let my_bytes = my_public.as_bytes();
        let their_bytes = their_public.as_bytes();
        let mut info = Vec::with_capacity(64 + b"rekindle-conversation-v1".len());
        if my_bytes < their_bytes {
            info.extend_from_slice(my_bytes);
            info.extend_from_slice(their_bytes);
        } else {
            info.extend_from_slice(their_bytes);
            info.extend_from_slice(my_bytes);
        }
        info.extend_from_slice(b"rekindle-conversation-v1");

        let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(&info, &mut key)
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
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        chacha20poly1305::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut output = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext produced by [`encrypt`].
    ///
    /// Expects `[24-byte nonce || ciphertext || 16-byte tag]`.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        use chacha20poly1305::XChaCha20Poly1305;
        use chacha20poly1305::XNonce;

        if data.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::DecryptionError(
                "ciphertext too short".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);

        let cipher = XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))
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
    fn dh_symmetry() {
        let alice_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let bob_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let alice_public = PublicKey::from(&alice_secret);
        let bob_public = PublicKey::from(&bob_secret);

        let key_a = DhtRecordKey::derive_conversation_key(&alice_secret, &bob_public);
        let key_b = DhtRecordKey::derive_conversation_key(&bob_secret, &alice_public);

        assert_eq!(key_a.key, key_b.key);
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

    #[test]
    fn conversation_key_from_identity() {
        use crate::Identity;

        let alice = Identity::generate();
        let bob = Identity::generate();

        let key_a = DhtRecordKey::derive_conversation_key(
            &alice.to_x25519_secret(),
            &bob.to_x25519_public(),
        );
        let key_b = DhtRecordKey::derive_conversation_key(
            &bob.to_x25519_secret(),
            &alice.to_x25519_public(),
        );

        assert_eq!(key_a.key, key_b.key);

        // Round-trip encryption between parties
        let msg = b"alice to bob";
        let ct = key_a.encrypt(msg).unwrap();
        let pt = key_b.decrypt(&ct).unwrap();
        assert_eq!(pt, msg);
    }
}
