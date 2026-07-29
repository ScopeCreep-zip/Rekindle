//! Clearance tiers — sensitivity classification on Peers and Frames.

/// Ordered sensitivity tiers. Higher numeric value = higher clearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Clearance {
    Unclassified = 0x00,
    Public       = 0x01,
    Internal     = 0x02,
    Confidential = 0x03,
    Restricted   = 0x04,
    Secret       = 0x05,
    TopSecret    = 0x06,
}

impl Clearance {
    pub const ALL: [Clearance; 7] = [
        Self::Unclassified,
        Self::Public,
        Self::Internal,
        Self::Confidential,
        Self::Restricted,
        Self::Secret,
        Self::TopSecret,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownClearance(pub u8);

impl TryFrom<u8> for Clearance {
    type Error = UnknownClearance;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Unclassified),
            0x01 => Ok(Self::Public),
            0x02 => Ok(Self::Internal),
            0x03 => Ok(Self::Confidential),
            0x04 => Ok(Self::Restricted),
            0x05 => Ok(Self::Secret),
            0x06 => Ok(Self::TopSecret),
            other => Err(UnknownClearance(other)),
        }
    }
}
