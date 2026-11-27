//! `VoicePacket` (schemas/voice_packet.capnp): the one voice wire format,
//! with what the sender signs and what the SFrame metadata authenticates.

use rekindle_types::domains::VOICE_PACKET;

use super::{bounds, capnp_err, pack, unpack_bounded, CodecError};
use crate::voice_packet_capnp::voice_packet;

/// One encrypted voice frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePacket {
    /// The sender's signing key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Per-sender frame sequence, for the jitter buffer.
    pub sequence: u32,
    /// Sender clock, milliseconds.
    pub timestamp: u64,
    /// RFC 9605 SFrame ciphertext (`header ‖ ciphertext`).
    pub sframe: Vec<u8>,
    /// Ed25519 signature over [`Self::signing_bytes`].
    pub sig: Vec<u8>,
}

impl VoicePacket {
    /// The packet fields outside the SFrame header, fed to SFrame as
    /// metadata so the ciphertext is bound to them (RFC 9605 §9.4): a
    /// relay cannot splice a frame under another sender or position.
    #[must_use]
    pub fn sframe_metadata(sender_key: &[u8], sequence: u32, timestamp: u64) -> Vec<u8> {
        [
            sender_key,
            &sequence.to_le_bytes(),
            &timestamp.to_le_bytes(),
        ]
        .concat()
    }

    /// The bytes the sender signs: the `VOICE_PACKET` domain, then every
    /// field but the signature.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        [
            VOICE_PACKET.as_bytes(),
            &Self::sframe_metadata(&self.sender_key, self.sequence, self.timestamp),
            &self.sframe,
        ]
        .concat()
    }

    /// Packed Cap'n Proto bytes.
        pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<voice_packet::Builder<'_>>();
            root.set_sender_key(&self.sender_key);
            root.set_sequence(self.sequence);
            root.set_timestamp(self.timestamp);
            root.set_sframe(&self.sframe);
            root.set_sig(&self.sig);
        }
        pack(&builder)
    }

    /// Decode packed Cap'n Proto bytes. The signature is not checked.
    pub fn decode(data: &[u8]) -> Result<Self, CodecError> {
        let reader = unpack_bounded(data, bounds::VOICE_PACKET_BOUND)?;
        let root = reader
            .get_root::<voice_packet::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;
        Ok(Self {
            sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
            sequence: root.get_sequence(),
            timestamp: root.get_timestamp(),
            sframe: root.get_sframe().map_err(|e| capnp_err(&e))?.to_vec(),
            sig: root.get_sig().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    }
}

impl super::SignedWire for VoicePacket {
    const WHAT: &'static str = "voice packet";
    fn signing_bytes(&self) -> Vec<u8> {
        VoicePacket::signing_bytes(self)
    }
    fn signer_key(&self) -> &[u8] {
        &self.sender_key
    }
    fn signature(&self) -> &[u8] {
        &self.sig
    }
    fn set_signature(&mut self, sig: Vec<u8>) {
        self.sig = sig;
    }
}

#[cfg(test)]
#[path = "voice_packet_tests.rs"]
mod tests;
