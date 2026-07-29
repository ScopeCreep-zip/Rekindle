//! FramePipeline — test helper for building and verifying complete wire packets.
//!
//! Wraps `FrameEncoder` + `FrameDecoder` with the same keys for loopback.
//! Not used in production — production code uses `FrameEncoder` and
//! `FrameDecoder` directly with correct per-direction keys.
//!
//! Build methods return `EncodedFrame` so tests can access both
//! session_seq (for audit chain correlation) and wire bytes.

use crate::v4::codec::aead::FrameCipher;
use crate::v4::crypto::keys::DerivedKeys;
use crate::v4::io::decode::{DecodeError, FrameDecoder};
use crate::v4::io::encode::{EncodedFrame, FrameEncoder};
use crate::v4::wire::outbound::OutboundFrame;
use crate::v4::wire::constants::{ENVELOPE_LEN, DIRECTION_ID_D2L};
use crate::v4::wire::frame_kind::StreamKind;
use crate::v4::wire::lane::Lane;

/// A decoded frame after full pipeline processing.
#[derive(Debug)]
pub struct PipelineFrame {
    pub lane: Lane,
    pub session_seq: u64,
    pub stream_id: Option<u8>,
    pub payload: Vec<u8>,
    /// True when the peer's epoch advanced on this frame — the first
    /// frame from the peer using a new key epoch after rotation.
    pub peer_epoch_advanced: bool,
}

/// Builds and receives complete wire packets through the full
/// EMAC → HeaderMAC → AEAD pipeline. Self-loopback test helper.
///
/// Uses a single direction (d2l) for both send and receive — this is a
/// self-loopback test helper, not production code. Production uses separate
/// FrameCipher instances per direction.
pub struct FramePipeline {
    encoder: FrameEncoder,
    decoder: FrameDecoder,
}

impl FramePipeline {
    /// Construct a loopback pipeline from derived keys.
    /// Both encoder and decoder use d2l direction for self-loopback.
    pub fn new(keys: &DerivedKeys) -> Self {
        let send_cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L)
            .expect("key init");
        let recv_cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L)
            .expect("key init");

        Self {
            encoder: FrameEncoder::new(
                keys.envelope_d2l, keys.header_d2l, send_cipher,
            ),
            decoder: FrameDecoder::new(
                keys.envelope_d2l, keys.header_d2l, recv_cipher,
            ),
        }
    }

    /// Build a Channel Lane packet (ChannelKind).
    pub fn build_channel_packet(
        &self, kind: crate::v4::wire::frame_kind::ChannelKind, payload: &[u8],
    ) -> EncodedFrame {
        self.encoder.encode(&OutboundFrame::Channel {
            kind, payload: payload.to_vec(),
        })
    }

    /// Build a Control Lane packet with raw class/kind for backward
    /// compatibility with tests that use raw u8 values.
    pub fn build_control_packet(
        &self, class: u8, kind: u8, payload: &[u8],
    ) -> EncodedFrame {
        use crate::v4::wire::frame_class::FrameClass;
        // Route to the correct typed variant based on class byte
        match class {
            x if x == FrameClass::Channel as u8 => {
                let k = crate::v4::wire::frame_kind::ChannelKind::try_from(kind)
                    .expect("invalid ChannelKind in build_control_packet");
                self.encoder.encode(&OutboundFrame::Channel { kind: k, payload: payload.to_vec() })
            }
            x if x == FrameClass::Datagram as u8 => {
                let k = crate::v4::wire::frame_kind::DatagramKind::try_from(kind)
                    .expect("invalid DatagramKind in build_control_packet");
                self.encoder.encode(&OutboundFrame::Datagram { kind: k, payload: payload.to_vec() })
            }
            _ => panic!("build_control_packet called with non-Control class: 0x{class:02x}"),
        }
    }

    /// Build a Data Lane packet with Stream Header.
    pub fn build_data_packet(
        &self, kind: StreamKind, stream_id: u8, chunk_index: u32, payload: &[u8],
    ) -> EncodedFrame {
        self.encoder.encode(&OutboundFrame::Data {
            stream_id, kind, chunk_index, payload: payload.to_vec(),
        })
    }

    /// Build an Audit Lane packet.
    pub fn build_audit_packet(
        &self, kind: crate::v4::wire::frame_kind::AuditKind, payload: &[u8],
    ) -> EncodedFrame {
        self.encoder.encode(&OutboundFrame::Audit {
            kind, payload: payload.to_vec(),
        })
    }

    /// Receive and verify a packet through the full pipeline.
    /// EMAC → HeaderMAC → AEAD verification, then returns decoded frame.
    /// Accepts `&[u8]`, `&Vec<u8>`, or `&EncodedFrame` via `AsRef<[u8]>`.
    pub fn receive_packet(
        &mut self, packet: impl AsRef<[u8]>,
    ) -> Result<PipelineFrame, DecodeError> {
        let packet = packet.as_ref();
        if packet.len() < ENVELOPE_LEN {
            return Err(DecodeError::FrameMalformed {
                lane: Lane::Control,
                body_len: 0,
                min: ENVELOPE_LEN as u32,
            });
        }

        let envelope_bytes: [u8; ENVELOPE_LEN] = packet[..ENVELOPE_LEN]
            .try_into().unwrap();
        let body = &packet[ENVELOPE_LEN..];

        let (decoded, peer_epoch_advanced) = self.decoder.decode_packet(&envelope_bytes, body)?;

        Ok(PipelineFrame {
            lane: decoded.envelope.lane,
            session_seq: decoded.envelope.session_seq,
            stream_id: decoded.header.map(|h| h.stream_id),
            payload: decoded.plaintext,
            peer_epoch_advanced,
        })
    }

    /// Access the encoder for session_seq counter reads and metrics.
    pub fn encoder(&self) -> &FrameEncoder {
        &self.encoder
    }

    /// Access the decoder for last_seen_seq reads.
    pub fn decoder(&self) -> &FrameDecoder {
        &self.decoder
    }

    /// Mutable access to the decoder for test setup (e.g., set_last_seen_seq).
    pub fn decoder_mut(&mut self) -> &mut FrameDecoder {
        &mut self.decoder
    }
}
