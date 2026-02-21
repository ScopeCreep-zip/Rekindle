use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

/// Media Encryption Key for group/channel message encryption.
///
/// Each community channel has its own MEK. It's distributed to members
/// via their individual Signal sessions and rotated on membership changes.
#[derive(ZeroizeOnDrop)]
pub struct MediaEncryptionKey {
    key: [u8; 32],
    /// Monotonically increasing generation number for key rotation tracking.
    #[zeroize(skip)]
    generation: u64,
}

impl MediaEncryptionKey {
    /// Generate a new random MEK.
    pub fn generate(generation: u64) -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self { key, generation }
    }

    /// Restore a MEK from raw bytes.
    pub fn from_bytes(key: [u8; 32], generation: u64) -> Self {
        Self { key, generation }
    }

    /// Get the raw key bytes (for encrypted distribution to members).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// Get the key generation number.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Encrypt a plaintext message.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

        // Prepend nonce to ciphertext
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Serialize to the 40-byte wire format: generation (8 LE) + key (32).
    ///
    /// Used for Stronghold persistence and network transport.
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(&self.generation.to_le_bytes());
        buf.extend_from_slice(&self.key);
        buf
    }

    /// Deserialize from the 40-byte wire format. Returns `None` if too short.
    pub fn from_wire_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            return None;
        }
        let generation = u64::from_le_bytes(bytes[..8].try_into().ok()?);
        let key: [u8; 32] = bytes[8..40].try_into().ok()?;
        Some(Self { key, generation })
    }

    /// Decrypt a ciphertext message (expects nonce prepended).
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::DecryptionError("data too short".into()));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;

        let nonce = Nonce::from_slice(&data[..12]);
        let ciphertext = &data[12..];

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mek = MediaEncryptionKey::generate(1);
        let plaintext = b"hello from a community channel";

        let encrypted = mek.encrypt(plaintext).unwrap();
        let decrypted = mek.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext.as_slice(), &decrypted);
    }

    #[test]
    fn wire_bytes_roundtrip() {
        let mek = MediaEncryptionKey::generate(42);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 40);

        let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn wire_bytes_too_short() {
        assert!(MediaEncryptionKey::from_wire_bytes(&[0u8; 39]).is_none());
        assert!(MediaEncryptionKey::from_wire_bytes(&[]).is_none());
    }

    #[test]
    fn different_keys_fail() {
        let mek1 = MediaEncryptionKey::generate(1);
        let mek2 = MediaEncryptionKey::generate(2);
        let plaintext = b"secret message";

        let encrypted = mek1.encrypt(plaintext).unwrap();
        assert!(mek2.decrypt(&encrypted).is_err());
    }
}
