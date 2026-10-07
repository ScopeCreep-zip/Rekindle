//! `VoicePacket` (schemas/voice_packet.capnp): the one voice wire format,
//! with what the sender signs and what the SFrame metadata authenticates.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rekindle_types::domains::VOICE_PACKET;

use super::{capnp_err, pack, unpack, ProtocolError};
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
    /// Transport-wide sequence for congestion feedback (0 until plan
    /// step E4 fills it).
    pub transport_seq: u64,
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
    pub fn sframe_metadata(
        sender_key: &[u8],
        sequence: u32,
        timestamp: u64,
        transport_seq: u64,
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(sender_key.len() + 20);
        out.extend_from_slice(sender_key);
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&timestamp.to_le_bytes());
        out.extend_from_slice(&transport_seq.to_le_bytes());
        out
    }

    /// The bytes the sender signs: the `VOICE_PACKET` domain, then every
    /// field but the signature.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(VOICE_PACKET.len() + self.sender_key.len() + 20 + self.sframe.len());
        out.extend_from_slice(VOICE_PACKET.as_bytes());
        out.extend_from_slice(&Self::sframe_metadata(
            &self.sender_key,
            self.sequence,
            self.timestamp,
            self.transport_seq,
        ));
        out.extend_from_slice(&self.sframe);
        out
    }

    /// Sign the packet with `key`, whose public half must be `sender_key`.
    pub fn sign(&mut self, key: &SigningKey) {
        self.sig = key.sign(&self.signing_bytes()).to_bytes().to_vec();
    }

    /// Check the signature against `sender_key`.
    pub fn verify(&self) -> Result<(), ProtocolError> {
        let key: [u8; 32] = self
            .sender_key
            .as_slice()
            .try_into()
            .map_err(|_| ProtocolError::Verification("voice sender key length".into()))?;
        let key = VerifyingKey::from_bytes(&key)
            .map_err(|e| ProtocolError::Verification(format!("voice sender key: {e}")))?;
        let sig: [u8; 64] = self
            .sig
            .as_slice()
            .try_into()
            .map_err(|_| ProtocolError::Verification("voice packet signature length".into()))?;
        key.verify_strict(&self.signing_bytes(), &Signature::from_bytes(&sig))
            .map_err(|e| ProtocolError::Verification(format!("voice packet signature: {e}")))
    }

    /// Packed Cap'n Proto bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<voice_packet::Builder<'_>>();
            root.set_sender_key(&self.sender_key);
            root.set_sequence(self.sequence);
            root.set_timestamp(self.timestamp);
            root.set_transport_seq(self.transport_seq);
            root.set_sframe(&self.sframe);
            root.set_sig(&self.sig);
        }
        pack(&builder)
    }

    /// Decode packed Cap'n Proto bytes. The signature is not checked.
    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        let reader = unpack(data)?;
        let root = reader
            .get_root::<voice_packet::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;
        Ok(Self {
            sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
            sequence: root.get_sequence(),
            timestamp: root.get_timestamp(),
            transport_seq: root.get_transport_seq(),
            sframe: root.get_sframe().map_err(|e| capnp_err(&e))?.to_vec(),
            sig: root.get_sig().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed() -> (VoicePacket, SigningKey) {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let mut packet = VoicePacket {
            sender_key: key.verifying_key().to_bytes().to_vec(),
            sequence: 7,
            timestamp: 1_000,
            transport_seq: 3,
            sframe: vec![0x12, 0xaa, 0xbb],
            sig: Vec::new(),
        };
        packet.sign(&key);
        (packet, key)
    }

    #[test]
    fn round_trip_and_verify() {
        let (packet, _) = signed();
        let decoded = VoicePacket::decode(&packet.encode()).unwrap();
        assert_eq!(decoded, packet);
        assert!(decoded.verify().is_ok());
    }

    #[test]
    fn every_signed_field_is_covered() {
        let (packet, _) = signed();
        let mutations: [fn(&mut VoicePacket); 4] = [
            |p| p.sequence += 1,
            |p| p.timestamp += 1,
            |p| p.transport_seq += 1,
            |p| p.sframe[0] ^= 1,
        ];
        for mutate in mutations {
            let mut tampered = packet.clone();
            mutate(&mut tampered);
            assert!(tampered.verify().is_err());
        }
    }
}
