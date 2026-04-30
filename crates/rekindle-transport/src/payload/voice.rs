//! Voice packet payload type.
//!
//! Voice packets are MEK-encrypted (AES-256-GCM), HMAC-BLAKE3 authenticated,
//! and Ed25519 signed by default. The signature can be opted out per-session
//! via [`VoiceAuthMode::TrustedHmacOnly`] for low-latency scenarios, but
//! HMAC authentication is always required. There is no mode that disables
//! both signature and HMAC.

use serde::{Deserialize, Serialize};

/// Voice packet authentication mode.
///
/// Defaults to `Signed` (Ed25519 + HMAC). Users may opt into `TrustedHmacOnly`
/// per-session for lower latency in trusted environments, but this is never
/// the default and cannot be persisted as a global setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceAuthMode {
    /// Full authentication: Ed25519 signature + BLAKE3 HMAC.
    /// Default. Provides sender authentication and per-packet integrity.
    #[default]
    Signed,
    /// Reduced authentication: BLAKE3 HMAC only, no Ed25519 signature.
    /// Lower latency but relies on the HMAC key (derived from MEK) for
    /// sender authentication. Only members with the channel MEK can
    /// produce valid HMACs, so this is safe within a trusted community
    /// but does not prove *which* member sent the packet.
    TrustedHmacOnly,
}

/// An encrypted voice packet for network transmission.
///
/// The `encrypted_audio` field contains AES-256-GCM encrypted Opus data.
/// The `hmac` field provides per-packet integrity using BLAKE3 keyed hash.
/// The `signature` field provides sender authentication via Ed25519.
///
/// In `TrustedHmacOnly` mode, `signature` is empty and the receiver skips
/// signature verification but still verifies the HMAC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePayload {
    /// Sender's Ed25519 public key (hex-encoded).
    pub sender_key_hex: String,
    /// Sequence number for ordering and jitter buffer insertion.
    pub sequence: u32,
    /// Timestamp in milliseconds (for jitter buffer timing).
    pub timestamp: u64,
    /// AES-256-GCM encrypted Opus audio data: `[12-byte nonce || ciphertext+tag]`.
    pub encrypted_audio: Vec<u8>,
    /// BLAKE3 HMAC (truncated to 16 bytes) over the encrypted audio data.
    /// Always present and always verified — there is no mode without HMAC.
    pub hmac: [u8; 16],
    /// Ed25519 signature over `sender_key_hex_bytes || sequence(4 LE) || timestamp(8 LE) || encrypted_audio`.
    /// Empty when `VoiceAuthMode::TrustedHmacOnly` is used for this session.
    /// Non-empty signatures are always verified; empty signatures are only
    /// accepted if the receiver's session is also in `TrustedHmacOnly` mode.
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl VoicePayload {
    /// Build the canonical bytes that the signature covers.
    ///
    /// Format: `sender_key_hex_bytes || sequence(4 LE) || timestamp(8 LE) || encrypted_audio`
    pub fn signature_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(
            self.sender_key_hex.len() + 4 + 8 + self.encrypted_audio.len(),
        );
        data.extend_from_slice(self.sender_key_hex.as_bytes());
        data.extend_from_slice(&self.sequence.to_le_bytes());
        data.extend_from_slice(&self.timestamp.to_le_bytes());
        data.extend_from_slice(&self.encrypted_audio);
        data
    }
}
