//! The voice packet wire format: the struct, what the sender signs,
//! and the per-packet AEAD for 1:1 calls.
//!
//! Split from the transport itself because these are the *bytes* — a
//! `VoicePacket` is meaningful without a transport, and the receiver
//! report codec next door reuses the same signing discipline under its
//! own domain tag.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use serde::{Deserialize, Serialize};

use crate::error::VoiceError;

// Wave 13 W13.14 — AEAD audio encryption under the X25519-derived
// call_key. ChaCha20-Poly1305 chosen for low-CPU (matters for mobile),
// constant-time (no timing oracles), large nonce space (12 bytes —
// no birthday attack at audio packet rates), and a Rust ecosystem
// implementation already used elsewhere in the project's crypto
// dependencies.

/// Domain-tag bytes that go into the nonce derivation so a chat or
/// governance ChaCha20-Poly1305 secret can never collide with a voice
/// nonce reused (defense-in-depth — the call_key is already
/// domain-separated by `derive_call_key`'s HKDF info).
const VOICE_AEAD_DOMAIN: &[u8; 4] = b"vca1";

/// Build a 12-byte nonce from `(sequence, timestamp)`. Each packet
/// gets a unique nonce because the (sequence, timestamp) pair is
/// strictly monotonic per call. Sender and receiver reconstruct the
/// same nonce from the public packet fields — no extra wire bytes.
fn aead_nonce(sequence: u32, timestamp: u64) -> [u8; 12] {
    // 4-byte domain tag + 4-byte sequence + 4 low-order bytes of
    // timestamp. (sequence, timestamp) is monotonic per call so the
    // resulting 12-byte nonce is unique even if timestamps roll over
    // every ~50 days at 48 kHz Opus framing — at which point sequence
    // alone is 32 bits of fresh space.
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(VOICE_AEAD_DOMAIN);
    nonce[4..8].copy_from_slice(&sequence.to_le_bytes());
    nonce[8..12].copy_from_slice(&((timestamp & 0xFFFF_FFFF) as u32).to_le_bytes());
    nonce
}

pub(super) fn encrypt_audio(
    call_key: &[u8; 32],
    sequence: u32,
    timestamp: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, VoiceError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(call_key));
    let nonce = aead_nonce(sequence, timestamp);
    cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| VoiceError::Transport(format!("aead encrypt: {e}")))
}

/// Receive-side decrypt entry point. Made public so receive_loop can
/// run the decrypt step after signature verification.
pub fn decrypt_packet_audio(
    call_key: &[u8; 32],
    packet: &VoicePacket,
) -> Result<Vec<u8>, VoiceError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(call_key));
    let nonce = aead_nonce(packet.sequence, packet.timestamp);
    cipher
        .decrypt(Nonce::from_slice(&nonce), packet.audio_data.as_slice())
        .map_err(|e| VoiceError::Transport(format!("aead decrypt: {e}")))
}

/// Voice packet for network transmission.
///
/// Architecture §10.3 + §26 W26 — `signature` is an Ed25519 signature
/// by the sender's pseudonym secret over [`signing_bytes`]. Receivers
/// MUST verify against `sender_key` before mixing/playing the audio,
/// otherwise any community member could MEK-encrypt audio claiming to
/// be any other (the MEK is community-shared and authenticates only
/// "some member encrypted this", not "this specific member").
#[derive(Clone, Serialize, Deserialize)]
pub struct VoicePacket {
    /// Sender public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Sequence number for ordering.
    pub sequence: u32,
    /// Timestamp in milliseconds.
    pub timestamp: u64,
    /// Opus-encoded audio data (MEK-encrypted ciphertext on the wire
    /// for community channels; plaintext for 1:1 calls).
    pub audio_data: Vec<u8>,
    /// Generation of the channel-media MEK that encrypted
    /// `audio_data` (0 for 1:1 calls — no MEK). Receivers holding a
    /// different generation drop the packet and fire the RequestMEK
    /// cascade instead of feeding garbage to the decoder.
    #[serde(default)]
    pub mek_generation: u64,
    /// 64-byte Ed25519 signature over [`signing_bytes`]. Receivers
    /// reject packets with an empty or invalid signature.
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl VoicePacket {
    /// Canonical bytes the sender signs. Domain-tagged so a signature
    /// for a chat or governance subkey can't be replayed as a voice
    /// packet (and vice versa).
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            b"rekindle-voice-packet-v1".len() + self.sender_key.len() + 20 + self.audio_data.len(),
        );
        out.extend_from_slice(b"rekindle-voice-packet-v1");
        out.extend_from_slice(&self.sender_key);
        out.extend_from_slice(&self.sequence.to_le_bytes());
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.mek_generation.to_le_bytes());
        out.extend_from_slice(&self.audio_data);
        out
    }
}
