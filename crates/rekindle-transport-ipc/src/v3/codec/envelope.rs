//! Envelope codec — serialize/deserialize the 32-byte outer Packet header.
//!
//! The EMAC (bytes 16..32) is a BLAKE3-keyed MAC over bytes 0..16.
//! Verification completes before any field is acted upon.
//!
//! `parse_envelope` implements RTI-SPEC-001 §6.4 steps 2-8:
//! 2. Verify EMAC
//! 3. Verify wire_version
//! 4. Verify lane is known
//! 5. Verify reserved flag bits are zero
//! 6. (session_seq monotonicity — caller responsibility, needs state)
//! 7. Verify body_len within lane max
//! 8. Verify body_len at or above lane min

use crate::v3::wire::constants::{
    WIRE_VERSION, ENVELOPE_LEN, EMAC_LEN,
    MAX_BODY_LEN_CONTROL, MAX_BODY_LEN_DATA, MAX_BODY_LEN_AUDIT, MAX_BODY_LEN_HANDOFF,
    MIN_BODY_LEN_CONTROL, MIN_BODY_LEN_DATA, MIN_BODY_LEN_AUDIT, MIN_BODY_LEN_HANDOFF,
};
use crate::v3::wire::envelope::{offsets, flags};
use crate::v3::wire::lane::{Lane, UnknownLane};

/// Domain-level envelope fields extracted after full verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvelopeInfo {
    pub wire_version: u8,
    pub lane: Lane,
    pub flags: u16,
    pub body_len: u32,
    pub session_seq: u64,
}

impl EnvelopeInfo {
    /// Extract the key epoch from the flags field (bit 4). Returns 0 or 1.
    /// The epoch bit is authenticated by EMAC inclusion — an attacker who
    /// flips it causes EMAC failure before this method is ever called.
    #[inline]
    pub fn key_epoch(&self) -> u8 {
        if self.flags & flags::KEY_EPOCH != 0 { 1 } else { 0 }
    }
}

/// Errors from envelope parsing — every variant is produced by
/// `parse_envelope`. Ordered by detection priority (EMAC first).
#[derive(Debug)]
pub enum EnvelopeError {
    /// EMAC verification failed — bytes were tampered or wrong key.
    /// No other field has been consulted.
    MacFailed,
    /// wire_version does not match supported version.
    WireVersionUnsupported(u8),
    /// Lane byte is not a recognized Lane value.
    LaneUnknown(u8),
    /// Reserved flag bits (4-15) are not zero.
    ReservedBitSet(u16),
    /// body_len exceeds the per-lane maximum.
    FrameTooLarge { lane: Lane, body_len: u32, max: u32 },
    /// body_len is below the per-lane minimum.
    FrameMalformed { lane: Lane, body_len: u32, min: u32 },
}

/// Build a 32-byte envelope with EMAC.
pub fn build_envelope(info: &EnvelopeInfo, envelope_key: &[u8; 32]) -> [u8; ENVELOPE_LEN] {
    let mut buf = [0u8; ENVELOPE_LEN];

    buf[offsets::WIRE_VERSION] = info.wire_version;
    buf[offsets::LANE] = info.lane as u8;
    buf[offsets::FLAGS..offsets::FLAGS + 2].copy_from_slice(&info.flags.to_le_bytes());
    buf[offsets::BODY_LEN..offsets::BODY_LEN + 4].copy_from_slice(&info.body_len.to_le_bytes());
    buf[offsets::SESSION_SEQ..offsets::SESSION_SEQ + 8]
        .copy_from_slice(&info.session_seq.to_le_bytes());

    let mac = compute_emac(&buf[..offsets::EMAC_INPUT_LEN], envelope_key);
    buf[offsets::ENVELOPE_MAC..offsets::ENVELOPE_MAC + EMAC_LEN].copy_from_slice(&mac);

    buf
}

/// Parse, verify, and validate a 32-byte envelope.
///
/// Implements §6.4 steps 2-5 and 7-8. Step 6 (session_seq monotonicity)
/// requires caller-maintained state and is the caller's responsibility.
///
/// On any tampering, returns `MacFailed` — the EMAC check precedes
/// all other checks. An attacker who modifies body_len gets `MacFailed`,
/// not `FrameTooLarge`.
pub fn parse_envelope(
    buf: &[u8; ENVELOPE_LEN],
    envelope_key: &[u8; 32],
) -> Result<EnvelopeInfo, EnvelopeError> {
    // Step 2: Verify EMAC before touching any field
    let expected_mac = compute_emac(&buf[..offsets::EMAC_INPUT_LEN], envelope_key);
    let actual_mac = &buf[offsets::ENVELOPE_MAC..offsets::ENVELOPE_MAC + EMAC_LEN];
    if !constant_time_eq(&expected_mac, actual_mac) {
        return Err(EnvelopeError::MacFailed);
    }

    // Step 3: wire_version
    let wire_version = buf[offsets::WIRE_VERSION];
    if wire_version != WIRE_VERSION {
        return Err(EnvelopeError::WireVersionUnsupported(wire_version));
    }

    // Step 4: lane
    let lane_byte = buf[offsets::LANE];
    let lane = Lane::try_from(lane_byte).map_err(|UnknownLane(v)| EnvelopeError::LaneUnknown(v))?;

    // Step 5: reserved flag bits
    let flag_bits = u16::from_le_bytes([buf[offsets::FLAGS], buf[offsets::FLAGS + 1]]);
    if flag_bits & flags::RESERVED_MASK != 0 {
        return Err(EnvelopeError::ReservedBitSet(flag_bits));
    }

    // Extract body_len and session_seq
    let body_len = u32::from_le_bytes([
        buf[offsets::BODY_LEN],
        buf[offsets::BODY_LEN + 1],
        buf[offsets::BODY_LEN + 2],
        buf[offsets::BODY_LEN + 3],
    ]);

    let session_seq = u64::from_le_bytes([
        buf[offsets::SESSION_SEQ],
        buf[offsets::SESSION_SEQ + 1],
        buf[offsets::SESSION_SEQ + 2],
        buf[offsets::SESSION_SEQ + 3],
        buf[offsets::SESSION_SEQ + 4],
        buf[offsets::SESSION_SEQ + 5],
        buf[offsets::SESSION_SEQ + 6],
        buf[offsets::SESSION_SEQ + 7],
    ]);

    // Step 7: body_len within lane max
    let (min, max) = lane_body_bounds(lane);
    if body_len > max {
        return Err(EnvelopeError::FrameTooLarge { lane, body_len, max });
    }

    // Step 8: body_len at or above lane min
    if body_len < min {
        return Err(EnvelopeError::FrameMalformed { lane, body_len, min });
    }

    Ok(EnvelopeInfo {
        wire_version,
        lane,
        flags: flag_bits,
        body_len,
        session_seq,
    })
}

/// Per-lane (min, max) body length bounds.
fn lane_body_bounds(lane: Lane) -> (u32, u32) {
    match lane {
        Lane::Control => (MIN_BODY_LEN_CONTROL, MAX_BODY_LEN_CONTROL),
        Lane::Data => (MIN_BODY_LEN_DATA, MAX_BODY_LEN_DATA),
        Lane::Audit => (MIN_BODY_LEN_AUDIT, MAX_BODY_LEN_AUDIT),
        Lane::Handoff => (MIN_BODY_LEN_HANDOFF, MAX_BODY_LEN_HANDOFF),
    }
}

fn compute_emac(input: &[u8], key: &[u8; 32]) -> [u8; EMAC_LEN] {
    let hash = blake3::keyed_hash(key, input);
    let mut mac = [0u8; EMAC_LEN];
    mac.copy_from_slice(&hash.as_bytes()[..EMAC_LEN]);
    mac
}

fn constant_time_eq(a: &[u8; EMAC_LEN], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}
