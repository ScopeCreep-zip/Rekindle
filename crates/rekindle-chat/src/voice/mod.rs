//! Voice — key derivation, packet encryption, HMAC authentication, session lifecycle.
//!
//! Voice session keys are derived from the channel's current MEK via
//! HKDF-SHA256 with a voice-specific info label. Key separation ensures
//! voice encryption keys rotate automatically when the channel MEK rotates.
//!
//! Packet flow:
//! 1. Opus frame → AES-256-GCM encrypt (this module)
//! 2. Encrypted frame + BLAKE3 HMAC → transport.send_to_peer (transport layer)
//!
//! On receive:
//! 1. Verify BLAKE3 HMAC (this module)
//! 2. AES-256-GCM decrypt → Opus frame (this module)
//! 3. Opus frame → audio playback (caller)

pub mod session;

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use aws_lc_rs::hkdf;
use aws_lc_rs::rand::SecureRandom;
use subtle::ConstantTimeEq;
use zeroize::ZeroizeOnDrop;

use crate::ChatError;

const VOICE_HKDF_INFO: &[u8] = b"rekindle-voice-session-v1";
const HMAC_LEN: usize = 16;
const TAG_LEN: usize = 16;

/// A derived voice session key (32 bytes, zeroized on drop).
///
/// Distinct from the MEK itself — provides key separation between
/// text encryption and voice encryption on the same channel.
#[derive(Clone, ZeroizeOnDrop)]
pub struct VoiceSessionKey {
    key: [u8; 32],
}

struct Len32;
impl hkdf::KeyType for Len32 {
    fn len(&self) -> usize { 32 }
}

impl VoiceSessionKey {
    /// Derive a voice session key from a channel MEK.
    pub fn derive_from_mek(mek_bytes: &[u8; 32]) -> Self {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
        let prk = salt.extract(mek_bytes);
        let okm = prk.expand(&[VOICE_HKDF_INFO], Len32)
            .expect("32-byte HKDF-SHA256 expand");
        let mut key = [0u8; 32];
        okm.fill(&mut key).expect("fill 32 bytes");
        Self { key }
    }

    /// Encrypt an Opus audio frame.
    ///
    /// Output: `[12-byte nonce || ciphertext || 16-byte GCM tag]`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, ChatError> {
        let unbound = UnboundKey::new(&AES_256_GCM, &self.key)
            .map_err(|e| ChatError::Internal(format!("voice key init: {e}")))?;
        let aead = LessSafeKey::new(unbound);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|e| ChatError::Internal(format!("voice nonce: {e}")))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = plaintext.to_vec();
        aead.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|e| ChatError::Internal(format!("voice encrypt: {e}")))?;

        let mut wire = Vec::with_capacity(NONCE_LEN + in_out.len());
        wire.extend_from_slice(&nonce_bytes);
        wire.extend_from_slice(&in_out);
        Ok(wire)
    }

    /// Decrypt an Opus audio frame.
    ///
    /// Input: `[12-byte nonce || ciphertext || 16-byte GCM tag]`.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, ChatError> {
        if data.len() < NONCE_LEN + TAG_LEN {
            return Err(ChatError::Internal("voice data too short".into()));
        }

        let unbound = UnboundKey::new(&AES_256_GCM, &self.key)
            .map_err(|e| ChatError::Internal(format!("voice key init: {e}")))?;
        let aead = LessSafeKey::new(unbound);

        let nonce_bytes: [u8; NONCE_LEN] = data[..NONCE_LEN]
            .try_into()
            .expect("slice len verified");
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = data[NONCE_LEN..].to_vec();
        let plaintext = aead
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| ChatError::Internal("voice decrypt: GCM tag failed".into()))?;
        Ok(plaintext.to_vec())
    }

    /// Compute a truncated BLAKE3 keyed HMAC for per-packet authentication.
    ///
    /// 16-byte output — faster than full AES-GCM tag verification for the
    /// high packet rate of voice (~50 packets/sec). Used as a lightweight
    /// pre-check before the more expensive GCM decrypt.
    pub fn compute_packet_hmac(&self, packet_data: &[u8]) -> [u8; HMAC_LEN] {
        let hash = blake3::keyed_hash(&self.key, packet_data);
        let mut hmac = [0u8; HMAC_LEN];
        hmac.copy_from_slice(&hash.as_bytes()[..HMAC_LEN]);
        hmac
    }

    /// Verify a BLAKE3 HMAC on a voice packet using constant-time comparison.
    pub fn verify_packet_hmac(
        &self,
        packet_data: &[u8],
        expected_hmac: &[u8; HMAC_LEN],
    ) -> bool {
        let computed = self.compute_packet_hmac(packet_data);
        bool::from(computed.ct_eq(expected_hmac))
    }
}

impl std::fmt::Debug for VoiceSessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSessionKey([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_encrypt_decrypt_roundtrip() {
        let mek = [42u8; 32];
        let key = VoiceSessionKey::derive_from_mek(&mek);
        let plaintext = b"opus frame data here";
        let encrypted = key.encrypt(plaintext).unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
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
        let hmac = key.compute_packet_hmac(data);
        assert!(key.verify_packet_hmac(data, &hmac));
    }

    #[test]
    fn hmac_rejects_tampered_data() {
        let key = VoiceSessionKey::derive_from_mek(&[7u8; 32]);
        let hmac = key.compute_packet_hmac(b"original");
        assert!(!key.verify_packet_hmac(b"tampered", &hmac));
    }

    #[test]
    fn different_meks_produce_different_session_keys() {
        let key1 = VoiceSessionKey::derive_from_mek(&[1u8; 32]);
        let key2 = VoiceSessionKey::derive_from_mek(&[2u8; 32]);
        assert_ne!(key1.key, key2.key);
    }
}
