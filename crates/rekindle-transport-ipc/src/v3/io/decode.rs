//! Inbound frame decoder — EMAC → bounds → monotonicity → HeaderMAC → AEAD.
//!
//! `FrameDecoder` is the single type that verifies and decrypts inbound
//! frames. It holds up to 2 epoch key sets (current + previous during
//! rotation transition). The epoch bit in the envelope flags selects
//! which key set to use for EMAC verification and AEAD decryption.
//!
//! Owned exclusively by the read task (single-threaded, &mut self).
//! No locking needed.

use crate::v3::audit::replay_filter::ReplayFilter;
use crate::v3::codec::aead::FrameCipher;
use crate::v3::codec::envelope::{self, EnvelopeInfo, EnvelopeError};
use crate::v3::codec::header::{self, StreamHeaderInfo};
use crate::v3::io::encode::EpochKeys;
use crate::v3::wire::constants::{ENVELOPE_LEN, STREAM_HEADER_LEN};
use crate::v3::wire::envelope::flags as env_flags;
use crate::v3::wire::lane::Lane;

/// Every possible failure from the inbound pipeline.
#[derive(Debug, Clone)]
pub enum DecodeError {
    EnvelopeMacFailed,
    WireVersionUnsupported(u8),
    LaneUnknown(u8),
    ReservedBitSet(u16),
    SequenceNonMonotonic { expected: u64, received: u64 },
    FrameTooLarge { lane: Lane, body_len: u32, max: u32 },
    FrameMalformed { lane: Lane, body_len: u32, min: u32 },
    HeaderMacFailed,
    AeadVerificationFailed,
    BodyTooShort { lane: Lane, body_len: usize },
    ReplayDetected { session_seq: u64 },
    /// Frame carries an epoch for which no keys are installed.
    /// This is a protocol violation or attack — fail-closed, never fallback.
    UnknownEpoch { epoch: u8 },
}

impl From<EnvelopeError> for DecodeError {
    fn from(e: EnvelopeError) -> Self {
        match e {
            EnvelopeError::MacFailed => Self::EnvelopeMacFailed,
            EnvelopeError::WireVersionUnsupported(v) => Self::WireVersionUnsupported(v),
            EnvelopeError::LaneUnknown(v) => Self::LaneUnknown(v),
            EnvelopeError::ReservedBitSet(f) => Self::ReservedBitSet(f),
            EnvelopeError::FrameTooLarge { lane, body_len, max } => {
                Self::FrameTooLarge { lane, body_len, max }
            }
            EnvelopeError::FrameMalformed { lane, body_len, min } => {
                Self::FrameMalformed { lane, body_len, min }
            }
        }
    }
}

/// A fully verified and decrypted inbound frame.
#[derive(Debug)]
pub struct DecodedFrame {
    pub envelope: EnvelopeInfo,
    pub header: Option<StreamHeaderInfo>,
    pub plaintext: Vec<u8>,
}

/// Decoder key storage — at most 2 key sets indexed by epoch & 1.
/// Slot 0 = even epochs (0, 2, ...), slot 1 = odd epochs (1, 3, ...).
struct DecoderKeySlots {
    slots: [Option<EpochKeys>; 2],
    current_epoch: u8,
}

impl DecoderKeySlots {
    fn new(initial_keys: EpochKeys) -> Self {
        let mut slots: [Option<EpochKeys>; 2] = [None, None];
        slots[0] = Some(initial_keys); // epoch 0 = slot 0
        Self { slots, current_epoch: 0 }
    }

    /// Select keys by epoch from the envelope. Returns None if the epoch
    /// has no installed keys — the frame must be rejected (fail-closed).
    fn keys_for_epoch(&self, epoch: u8) -> Option<&EpochKeys> {
        self.slots[(epoch & 1) as usize].as_ref()
    }

    /// Install keys for a new epoch. The slot is epoch & 1.
    /// Any previous occupant (2 epochs ago) is dropped.
    fn install(&mut self, epoch: u8, keys: EpochKeys) {
        let slot = (epoch & 1) as usize;
        let had_previous = self.slots[slot].is_some();
        let new_key_fp = hex::encode(&keys.envelope_key[..8]);
        tracing::debug!(
            epoch,
            slot,
            had_previous,
            old_epoch = self.current_epoch,
            new_key_fp = %new_key_fp,
            "DecoderKeySlots::install"
        );
        self.slots[slot] = Some(keys);
        self.current_epoch = epoch;
    }

    // Retirement is slot overwrite — install(epoch, keys) drops the
    // previous occupant of slot[epoch & 1]. No explicit retire method.
}

/// Inbound frame verification and decryption.
///
/// Holds up to 2 epoch key sets. The epoch bit in the envelope flags
/// selects which key set to use. Owned exclusively by the read task.
pub struct FrameDecoder {
    key_slots: DecoderKeySlots,
    last_seen_seq: Option<u64>,
    last_peer_epoch: u8,
    replay: ReplayFilter,
}

impl FrameDecoder {
    /// Construct a decoder with initial epoch=0 keys.
    pub fn new(
        envelope_key: [u8; 32],
        header_key: [u8; 32],
        cipher: FrameCipher,
    ) -> Self {
        Self {
            key_slots: DecoderKeySlots::new(EpochKeys {
                envelope_key, header_key, cipher,
            }),
            last_seen_seq: None,
            last_peer_epoch: 0,
            replay: ReplayFilter::new(),
        }
    }

    /// Install keys for a new epoch. Called by the read task when it
    /// receives an install signal from the control loop via EpochSignal.
    pub fn install_epoch_keys(&mut self, epoch: u8, keys: EpochKeys) {
        self.key_slots.install(epoch, keys);
    }

    // Retirement is slot overwrite — no explicit retire method needed.
    // install_epoch_keys(epoch, keys) drops the previous slot occupant
    // (epoch-2 keys) via Option::replace. ZeroizeOnDrop fires.

    /// Verify a 32-byte Envelope. Selects the key set by the epoch bit
    /// in the flags field (read from plaintext, authenticated by EMAC).
    ///
    /// Returns the EnvelopeInfo and whether the peer's epoch advanced
    /// (first frame from the peer with a new epoch — retirement signal).
    pub fn verify_envelope(
        &mut self,
        envelope_bytes: &[u8; ENVELOPE_LEN],
    ) -> Result<(EnvelopeInfo, bool), DecodeError> {
        // Extract epoch from plaintext flags BEFORE EMAC verification.
        // The epoch bit is authenticated by EMAC inclusion — an attacker
        // who flips it causes EMAC failure. Fail-closed.
        let flag_bits = u16::from_le_bytes([
            envelope_bytes[crate::v3::wire::envelope::offsets::FLAGS],
            envelope_bytes[crate::v3::wire::envelope::offsets::FLAGS + 1],
        ]);
        let frame_epoch = if flag_bits & env_flags::KEY_EPOCH != 0 { 1u8 } else { 0u8 };

        // Select envelope key by epoch — fail-closed on unknown epoch
        let keys = self.key_slots.keys_for_epoch(frame_epoch)
            .ok_or_else(|| {
                tracing::error!(frame_epoch, decoder_epoch = self.key_slots.current_epoch, "verify_envelope: UnknownEpoch — no keys installed for this epoch");
                DecodeError::UnknownEpoch { epoch: frame_epoch }
            })?;

        let key_fp = hex::encode(&keys.envelope_key[..8]);
        let info = match envelope::parse_envelope(envelope_bytes, &keys.envelope_key) {
            Ok(info) => info,
            Err(e) => {
                tracing::error!(
                    frame_epoch,
                    decoder_epoch = self.key_slots.current_epoch,
                    key_fp = %key_fp,
                    error = ?e,
                    "verify_envelope: EMAC failed"
                );
                return Err(e.into());
            }
        };

        // Replay check — after EMAC, before body read or routing.
        if self.replay.check_and_accept(info.session_seq).is_err() {
            return Err(DecodeError::ReplayDetected { session_seq: info.session_seq });
        }

        // Detect peer epoch advancement — retirement signal
        let peer_advanced = if frame_epoch != self.last_peer_epoch {
            self.last_peer_epoch = frame_epoch;
            true
        } else {
            false
        };

        self.last_seen_seq = Some(info.session_seq);
        Ok((info, peer_advanced))
    }

    /// Verify and decrypt the frame body using the epoch from the envelope.
    pub fn decode_body(
        &self,
        envelope_bytes: &[u8; ENVELOPE_LEN],
        envelope_info: &EnvelopeInfo,
        body: &[u8],
    ) -> Result<DecodedFrame, DecodeError> {
        let frame_epoch = envelope_info.key_epoch();
        let keys = self.key_slots.keys_for_epoch(frame_epoch)
            .ok_or_else(|| {
                tracing::error!(
                    frame_epoch,
                    decoder_epoch = self.key_slots.current_epoch,
                    slot0_present = self.key_slots.slots[0].is_some(),
                    slot1_present = self.key_slots.slots[1].is_some(),
                    session_seq = envelope_info.session_seq,
                    "decode_body: UnknownEpoch — no keys for frame epoch"
                );
                DecodeError::UnknownEpoch { epoch: frame_epoch }
            })?;

        match envelope_info.lane {
            Lane::Data => {
                if body.len() < STREAM_HEADER_LEN {
                    return Err(DecodeError::BodyTooShort {
                        lane: Lane::Data,
                        body_len: body.len(),
                    });
                }

                let header_bytes: &[u8; STREAM_HEADER_LEN] = body[..STREAM_HEADER_LEN]
                    .try_into()
                    .expect("STREAM_HEADER_LEN is 32");

                let header_info = header::parse_header(header_bytes, &keys.header_key)
                    .map_err(|_| DecodeError::HeaderMacFailed)?;

                let ciphertext_and_tag = &body[STREAM_HEADER_LEN..];
                let plaintext = keys.cipher.open(
                    header_info.nonce,
                    envelope_bytes,
                    Some(header_bytes),
                    ciphertext_and_tag,
                ).map_err(|_| DecodeError::AeadVerificationFailed)?;

                Ok(DecodedFrame {
                    envelope: *envelope_info,
                    header: Some(header_info),
                    plaintext,
                })
            }
            _ => {
                let plaintext = keys.cipher.open(
                    envelope_info.session_seq,
                    envelope_bytes,
                    None,
                    body,
                ).map_err(|_| DecodeError::AeadVerificationFailed)?;

                Ok(DecodedFrame {
                    envelope: *envelope_info,
                    header: None,
                    plaintext,
                })
            }
        }
    }

    /// One-shot: verify envelope + decode body.
    pub fn decode_packet(
        &mut self,
        envelope_bytes: &[u8; ENVELOPE_LEN],
        body: &[u8],
    ) -> Result<(DecodedFrame, bool), DecodeError> {
        let (info, peer_advanced) = self.verify_envelope(envelope_bytes)?;
        let decoded = self.decode_body(envelope_bytes, &info, body)?;
        Ok((decoded, peer_advanced))
    }

    /// The cipher for the current epoch. Needed by BulkReceiver.
    pub fn cipher(&self) -> &FrameCipher {
        &self.key_slots.keys_for_epoch(self.key_slots.current_epoch)
            .expect("current epoch must always be populated")
            .cipher
    }

    /// Last verified session_seq.
    pub fn last_seen_seq(&self) -> Option<u64> {
        self.last_seen_seq
    }

    /// The envelope key for the current epoch.
    pub fn envelope_key(&self) -> &[u8; 32] {
        &self.key_slots.keys_for_epoch(self.key_slots.current_epoch)
            .expect("current epoch must always be populated")
            .envelope_key
    }

    /// The header key for the current epoch.
    pub fn header_key(&self) -> &[u8; 32] {
        &self.key_slots.keys_for_epoch(self.key_slots.current_epoch)
            .expect("current epoch must always be populated")
            .header_key
    }

    /// Current epoch number — for debug_assert synchronization with BulkReceiver.
    pub fn current_epoch(&self) -> u8 { self.key_slots.current_epoch }

    /// Set last-seen seq for test setup.
    pub fn set_last_seen_seq(&mut self, seq: u64) {
        self.last_seen_seq = Some(seq);
    }
}
