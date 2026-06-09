//! FrameClass — first byte of decrypted payload (or Header for Data Lane).

/// The protocol role of a Frame, determined by the first byte after
/// decryption (Control/Audit/Handoff) or the first byte of the
/// Stream Header (Data Lane).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameClass {
    Channel  = 0x01,
    Stream   = 0x02,
    Datagram = 0x03,
    Audit    = 0x04,
    Handoff  = 0x05,
}

impl FrameClass {
    pub const ALL: [FrameClass; 5] = [
        FrameClass::Channel,
        FrameClass::Stream,
        FrameClass::Datagram,
        FrameClass::Audit,
        FrameClass::Handoff,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownFrameClass(pub u8);

impl TryFrom<u8> for FrameClass {
    type Error = UnknownFrameClass;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(FrameClass::Channel),
            0x02 => Ok(FrameClass::Stream),
            0x03 => Ok(FrameClass::Datagram),
            0x04 => Ok(FrameClass::Audit),
            0x05 => Ok(FrameClass::Handoff),
            other => Err(UnknownFrameClass(other)),
        }
    }
}
