//! Lane discriminant — the first routing decision on every Packet.

/// Lane discriminant byte in the Envelope at offset 1.
///
/// Determines which cryptographic context decrypts the Frame body,
/// which dispatch path handles it, and which resource accounting applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Lane {
    /// Channel (Class C) and Datagram (Class D) frames.
    /// Stateful Noise transport. Highest write priority.
    Control = 0x01,
    /// Stream (Class S) frames. Stateless per-frame AEAD.
    /// Lowest write priority.
    Data = 0x02,
    /// Audit (Class A) frames. Stateful Noise transport.
    Audit = 0x03,
    /// Handoff (Class H) frames. Stateful Noise transport.
    Handoff = 0x04,
}

impl Lane {
    /// All valid Lane variants in wire-priority order (highest first).
    pub const ALL: [Lane; 4] = [
        Lane::Control,
        Lane::Audit,
        Lane::Handoff,
        Lane::Data,
    ];
}

/// Error returned when a byte does not map to a known Lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownLane(pub u8);

impl TryFrom<u8> for Lane {
    type Error = UnknownLane;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Lane::Control),
            0x02 => Ok(Lane::Data),
            0x03 => Ok(Lane::Audit),
            0x04 => Ok(Lane::Handoff),
            other => Err(UnknownLane(other)),
        }
    }
}

/// Map the envelope's wire lane to the processing lane.
///
/// Most frames are processed by the lane indicated in the envelope.
/// Three Channel-class frames modify Data lane state and must be
/// rerouted to the Data lane for correct ownership under lane sharding:
///
/// - Credit (0x01, 0x09) — modifies stream_credits, lane_credit_bytes
/// - BackpressureAssert (0x01, 0x0A) — modifies BackpressureState
/// - BackpressureClear (0x01, 0x0B) — modifies BackpressureState
///
/// `plaintext` is the decrypted frame body. For non-Data lanes, the
/// first two bytes are [class, kind]. This function is called after
/// AEAD decryption, before lane channel routing.
///
/// Returns the processing lane — may differ from the envelope lane.
#[inline]
pub fn processing_lane(envelope_lane: Lane, plaintext: &[u8]) -> Lane {
    if envelope_lane == Lane::Control && plaintext.len() >= 2 {
        let class = plaintext[0];
        let kind = plaintext[1];
        if class == 0x01 && matches!(kind, 0x09 | 0x0A | 0x0B) {
            return Lane::Data;
        }
    }
    envelope_lane
}
