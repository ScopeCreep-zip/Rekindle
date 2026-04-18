//! Secret key wrapper types with Zeroize guarantees.
//!
//! Every secret in Rekindle passes through one of these types.
//! All implement `ZeroizeOnDrop` — memory is scrubbed when the value drops.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use rekindle_types::error::CryptoError;
use zeroize::ZeroizeOnDrop;

/// A user's master secret — the root of all identity derivation.
/// Stored in Stronghold, never leaves the device.
#[derive(Clone, ZeroizeOnDrop)]
pub struct MasterSecret(pub [u8; 32]);

/// Shared secret used to derive SMPL slot keypairs for a community.
/// Distributed to all members via InviteSecrets.
#[derive(Clone, ZeroizeOnDrop)]
pub struct SlotSeed(pub [u8; 32]);

impl SlotSeed {
    /// Generate a cryptographically random slot seed.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
}

/// Media Encryption Key for group/channel message encryption.
///
/// Each channel has its own MEK. Rotated on membership changes via the
/// deterministic rotator protocol (peer-to-peer, no coordinator).
#[derive(ZeroizeOnDrop)]
pub struct MediaEncryptionKey {
    key: [u8; 32],
    /// Monotonically increasing generation number for rotation tracking.
    #[zeroize(skip)]
    generation: u64,
}

impl MediaEncryptionKey {
    /// Generate a new random MEK at the given generation.
    pub fn generate(generation: u64) -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self { key, generation }
    }

    /// Restore a MEK from raw bytes and generation.
    pub fn from_bytes(key: [u8; 32], generation: u64) -> Self {
        Self { key, generation }
    }

    /// Get the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.key
    }

    /// Get the generation number.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Encrypt plaintext with AES-256-GCM.
    ///
    /// Output format: `[12-byte nonce || ciphertext + 16-byte tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt ciphertext (expects `[12-byte nonce || ciphertext + tag]`).
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 12 {
            return Err(CryptoError::Decryption("data too short for nonce".into()));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::Decryption(e.to_string()))?;

        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| CryptoError::Decryption(e.to_string()))
    }

    /// Serialize to the 40-byte wire format: `[generation LE (8) || key (32)]`.
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
}

/// Ephemeral symmetric key for a direct or group voice/video call.
#[derive(Clone, ZeroizeOnDrop)]
pub struct CallKey(pub [u8; 32]);

impl CallKey {
    /// Generate a random call key.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mek_encrypt_decrypt_roundtrip() {
        let mek = MediaEncryptionKey::generate(1);
        let plaintext = b"hello from a community channel";
        let encrypted = mek.encrypt(plaintext).unwrap();
        let decrypted = mek.decrypt(&encrypted).unwrap();
        assert_eq!(plaintext.as_slice(), &decrypted);
    }

    #[test]
    fn mek_wire_bytes_roundtrip() {
        let mek = MediaEncryptionKey::generate(42);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 40);
        let restored = MediaEncryptionKey::from_wire_bytes(&wire).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn mek_wire_bytes_too_short() {
        assert!(MediaEncryptionKey::from_wire_bytes(&[0u8; 39]).is_none());
        assert!(MediaEncryptionKey::from_wire_bytes(&[]).is_none());
    }

    #[test]
    fn different_keys_fail_decrypt() {
        let mek1 = MediaEncryptionKey::generate(1);
        let mek2 = MediaEncryptionKey::generate(2);
        let encrypted = mek1.encrypt(b"secret").unwrap();
        assert!(mek2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn slot_seed_generates_random() {
        let s1 = SlotSeed::generate();
        let s2 = SlotSeed::generate();
        assert_ne!(s1.0, s2.0);
    }
}
