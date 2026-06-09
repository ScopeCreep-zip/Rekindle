//! AUDIT_REPLAY — selective retransmission, verification, and chain grafting.
//!
//! `verify_and_graft` verifies replayed packets through the full
//! EMAC → HeaderMAC → AEAD pipeline via `FrameDecoder::decode_body`,
//! then computes LinkInputs and grafts them into the audit chain.
//!
//! Replay verification deliberately skips session_seq monotonicity
//! (§6.4 step 6) because replayed packets carry their original
//! session_seq values which are older than the current chain position.

use crate::v3::codec::envelope;
use crate::v3::io::decode::FrameDecoder;
use crate::v3::wire::constants::{ENVELOPE_LEN, STREAM_HEADER_LEN};
use crate::v3::wire::lane::Lane;

use super::chain::{AuditChain, LinkInput};
use super::gap::MissingBitmap;
use super::retention::RetentionBuffer;

/// Result of building a replay from a retention buffer.
#[derive(Debug)]
pub enum ReplayResult {
    Frames(Vec<Vec<u8>>),
    Unfillable { missing_seqs: Vec<u64> },
}

/// Build a replay containing only the frames marked missing in the bitmap.
pub fn build_replay(
    retention: &RetentionBuffer,
    gap_start: u64,
    gap_end: u64,
    bitmap: &MissingBitmap,
) -> ReplayResult {
    let mut frames = Vec::new();
    let mut unfillable = Vec::new();

    for seq in bitmap.missing_seqs() {
        if seq < gap_start || seq > gap_end {
            continue;
        }
        match retention.get(seq) {
            Some(frame) => frames.push(frame.to_vec()),
            None => unfillable.push(seq),
        }
    }

    if unfillable.is_empty() {
        ReplayResult::Frames(frames)
    } else {
        ReplayResult::Unfillable { missing_seqs: unfillable }
    }
}

#[derive(Debug)]
pub enum GraftError {
    AeadVerificationFailed { session_seq: u64 },
    EnvelopeMacFailed { session_seq: u64 },
    HeaderMacFailed { session_seq: u64 },
    PacketTooShort { session_seq: u64 },
}

/// Graft missing frames into a chain using pre-verified LinkInputs.
///
/// Pure chain operation — does not verify frame integrity. The caller
/// verifies each frame before extracting the LinkInput. Use
/// `verify_and_graft` for the production path.
pub fn graft_frames(
    chain: &mut AuditChain,
    missing_frames: &[LinkInput],
    post_gap_frames: &[LinkInput],
) {
    let mut all: Vec<LinkInput> = Vec::with_capacity(missing_frames.len() + post_gap_frames.len());
    all.extend_from_slice(missing_frames);
    all.extend_from_slice(post_gap_frames);
    all.sort_by_key(|input| input.session_seq);
    for input in &all {
        chain.advance(*input);
    }
}

/// Verify replayed packets through the full EMAC → HeaderMAC → AEAD
/// pipeline, extract LinkInputs, and graft them into the chain.
///
/// Uses `FrameDecoder::decode_body` for HeaderMAC + AEAD verification.
/// Skips `verify_envelope` (which enforces monotonicity) because
/// replayed packets carry original session_seq values.
///
/// Each packet must be a complete wire Packet (Envelope + Body) as
/// originally emitted. If any packet fails verification, the graft
/// is aborted at that point.
pub fn verify_and_graft(
    chain: &mut AuditChain,
    missing_packets: &[Vec<u8>],
    post_gap_inputs: &[LinkInput],
    decoder: &FrameDecoder,
) -> Result<(), GraftError> {
    for packet in missing_packets {
        let input = verify_packet(packet, decoder)?;
        chain.advance(input);
    }
    for input in post_gap_inputs {
        chain.advance(*input);
    }
    Ok(())
}

/// Verify a single retained Packet and extract its LinkInput.
///
/// EMAC is verified via `parse_envelope` (no monotonicity state needed).
/// HeaderMAC + AEAD are verified via `decoder.decode_body` which handles
/// both Data lane (Header + AEAD) and non-Data lane (AEAD only) correctly.
fn verify_packet(
    packet: &[u8],
    decoder: &FrameDecoder,
) -> Result<LinkInput, GraftError> {
    if packet.len() < ENVELOPE_LEN {
        return Err(GraftError::PacketTooShort { session_seq: 0 });
    }

    let envelope_bytes: [u8; ENVELOPE_LEN] = packet[..ENVELOPE_LEN]
        .try_into()
        .map_err(|_| GraftError::PacketTooShort { session_seq: 0 })?;

    // EMAC verification — no monotonicity check (replay packets have old seq)
    let env_info = envelope::parse_envelope(&envelope_bytes, decoder.envelope_key())
        .map_err(|_| GraftError::EnvelopeMacFailed { session_seq: 0 })?;

    let body = &packet[ENVELOPE_LEN..];

    // HeaderMAC + AEAD verification via FrameDecoder
    decoder.decode_body(&envelope_bytes, &env_info, body)
        .map_err(|e| {
            use crate::v3::io::decode::DecodeError;
            match e {
                DecodeError::HeaderMacFailed => {
                    GraftError::HeaderMacFailed { session_seq: env_info.session_seq }
                }
                DecodeError::AeadVerificationFailed => {
                    GraftError::AeadVerificationFailed { session_seq: env_info.session_seq }
                }
                DecodeError::BodyTooShort { .. } => {
                    GraftError::PacketTooShort { session_seq: env_info.session_seq }
                }
                _ => GraftError::AeadVerificationFailed { session_seq: env_info.session_seq },
            }
        })?;

    // Compute hashes for the audit chain LinkInput
    let envelope_hash = *blake3::hash(&envelope_bytes).as_bytes();
    let header_hash = if env_info.lane == Lane::Data && body.len() >= STREAM_HEADER_LEN {
        *blake3::hash(&body[..STREAM_HEADER_LEN]).as_bytes()
    } else {
        [0u8; 32]
    };
    let ciphertext_hash = *blake3::hash(body).as_bytes();

    Ok(LinkInput {
        session_seq: env_info.session_seq,
        envelope_hash,
        header_hash,
        ciphertext_hash,
    })
}
