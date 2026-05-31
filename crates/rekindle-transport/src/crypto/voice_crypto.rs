//! Voice packet encryption and per-packet authentication.
//!
//! Voice packets are MEK-encrypted (AES-256-GCM) for confidentiality and
//! HMAC-BLAKE3 authenticated for lightweight per-packet integrity.
//!
//! The voice session key is derived from the channel's current MEK via
//! HKDF-SHA256 with a voice-specific info label. This ensures voice
//! encryption keys rotate automatically when the channel MEK rotates.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use zeroize::ZeroizeOnDrop;

use crate::error::{Result, TransportError};

/// HKDF info label for deriving voice session keys from channel MEK.
const VOICE_HKDF_INFO: &[u8] = b"rekindle-voice-session-v1";

/// A derived voice session key (32 bytes, zeroized on drop).
#[derive(Clone, ZeroizeOnDrop)]
pub struct VoiceSessionKey {
    key: [u8; 32],
}

impl VoiceSessionKey {
    /// Derive a voice session key from a channel MEK.
    ///
    /// Uses HKDF-SHA256 with a voice-specific info label. The session key
    /// is distinct from the MEK itself, providing key separation between
    /// text encryption and voice encryption on the same channel.
    pub fn derive_from_mek(mek_bytes: &[u8; 32]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(None, mek_bytes);
        let mut key = [0u8; 32];
        hkdf.expand(VOICE_HKDF_INFO, &mut key)
            .expect("32-byte output is valid for HKDF-SHA256");
        Self { key }
    }

    /// Encrypt an Opus audio frame.
    ///
    /// Returns `[12-byte nonce || ciphertext+tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| TransportError::EncryptionFailed {
                reason: e.to_string(),
            })?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext =
            cipher
                .encrypt(nonce, plaintext)
                .map_err(|e| TransportError::EncryptionFailed {
                    reason: e.to_string(),
                })?;

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypt an Opus audio frame.
    ///
    /// Expects `[12-byte nonce || ciphertext+tag]` format.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(TransportError::DecryptionFailed {
                reason: "voice data too short for nonce".into(),
            });
        }

        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|e| TransportError::DecryptionFailed {
                reason: e.to_string(),
            })?;

        let nonce = Nonce::from_slice(&data[..12]);
        let ciphertext = &data[12..];

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| TransportError::DecryptionFailed {
                reason: format!("voice AES-GCM decrypt: {e}"),
            })
    }
}

/// Compute a BLAKE3 HMAC for lightweight per-packet authentication.
///
/// The HMAC key is derived from the voice session key via BLAKE3 keyed hash.
/// This is faster than a full AES-GCM tag verification and suitable for
/// the high packet rate of voice (~50 packets/sec).
pub fn compute_packet_hmac(session_key: &VoiceSessionKey, packet_data: &[u8]) -> [u8; 16] {
    let hash = blake3::keyed_hash(&session_key.key, packet_data);
    let bytes = hash.as_bytes();
    let mut hmac = [0u8; 16];
    hmac.copy_from_slice(&bytes[..16]);
    hmac
}

/// Verify a BLAKE3 HMAC on a voice packet using constant-time comparison.
///
/// Uses XOR + OR accumulation to prevent timing side-channels. The comparison
/// always examines all 16 bytes regardless of where a mismatch occurs.
pub fn verify_packet_hmac(
    session_key: &VoiceSessionKey,
    packet_data: &[u8],
    expected_hmac: &[u8; 16],
) -> bool {
    let computed = compute_packet_hmac(session_key, packet_data);
    constant_time_eq(&computed, expected_hmac)
}

/// Constant-time comparison of two 16-byte arrays.
///
/// Examines all bytes regardless of mismatch position. Prevents timing
/// oracle attacks that could recover the HMAC byte-by-byte.
fn constant_time_eq(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut acc: u8 = 0;
    for i in 0..16 {
        acc |= a[i] ^ b[i];
    }
    acc == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_encrypt_decrypt_roundtrip() {
        let mek = [42u8; 32];
        let session_key = VoiceSessionKey::derive_from_mek(&mek);

        let plaintext = b"opus frame data here";
        let encrypted = session_key.encrypt(plaintext).unwrap();
        let decrypted = session_key.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails_decrypt() {
        let key1 = VoiceSessionKey::derive_from_mek(&[1u8; 32]);
        let key2 = VoiceSessionKey::derive_from_mek(&[2u8; 32]);

        let encrypted = key1.encrypt(b"secret audio").unwrap();
        assert!(key2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn hmac_verify_roundtrip() {
        let key = VoiceSessionKey::derive_from_mek(&[7u8; 32]);
        let data = b"packet payload";
        let hmac = compute_packet_hmac(&key, data);
        assert!(verify_packet_hmac(&key, data, &hmac));
    }

    #[test]
    fn hmac_rejects_tampered_data() {
        let key = VoiceSessionKey::derive_from_mek(&[7u8; 32]);
        let hmac = compute_packet_hmac(&key, b"original");
        assert!(!verify_packet_hmac(&key, b"tampered", &hmac));
    }

    #[test]
    fn different_meks_produce_different_session_keys() {
        let key1 = VoiceSessionKey::derive_from_mek(&[1u8; 32]);
        let key2 = VoiceSessionKey::derive_from_mek(&[2u8; 32]);
        assert_ne!(key1.key, key2.key);
    }
}
