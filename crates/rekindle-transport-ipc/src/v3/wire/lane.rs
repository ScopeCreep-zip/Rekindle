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
