//! The voice packet wire format lives in `rekindle-protocol`
//! (`schemas/voice_packet.capnp`); this module is its home in the voice
//! crate's namespace, plus the outbound frame the transport signs.

pub use rekindle_codec::capnp_codec::voice_packet::VoicePacket;

/// One sealed frame on its way out: the packet fields the transport signs
/// around the SFrame ciphertext.
#[derive(Debug, Clone)]
pub struct OutboundFrame {
    pub sequence: u32,
    pub timestamp: u64,
    pub sframe: Vec<u8>,
    /// Opus bytes the frame carries, for the route's cost accounting.
    pub media_bytes: usize,
}
