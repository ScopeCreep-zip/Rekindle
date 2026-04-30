//! Voice packet payload type.
//!
//! Voice packets are MEK-encrypted (AES-256-GCM) and HMAC-BLAKE3
//! authenticated at the transport layer. No raw Opus data on the wire.

use serde::{Deserialize, Serialize};

/// An encrypted voice packet for network transmission.
///
/// The `encrypted_audio` field contains AES-256-GCM encrypted Opus data.
/// The `hmac` field provides lightweight per-packet authentication using
/// BLAKE3 keyed hash derived from the voice session key.
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
    /// Computed with the voice session key derived from the channel MEK.
    pub hmac: [u8; 16],
}
