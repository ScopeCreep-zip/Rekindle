//! FrameKind enums — per-class dispatch discriminants.
//!
//! Each enum has an `all_variants()` method used by exhaustiveness
//! tests to verify the dispatch table covers every defined kind.

/// Channel (Class C) frame kinds on the Control Lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChannelKind {
    Hello              = 0x01,
    HelloAck           = 0x02,
    Goodbye            = 0x03,
    GoodbyeAck         = 0x04,
    Ping               = 0x05,
    Pong               = 0x06,
    Ack                = 0x07,
    Nack               = 0x08,
    Credit             = 0x09,
    Backpressure       = 0x0A,
    BackpressureClear  = 0x0B,
    RotateInit         = 0x0C,
    RotateCommit       = 0x0D,
    Revoke             = 0x0E,
    ChannelError       = 0x0F,
    Subscribe          = 0x10,
    SubscribeAck       = 0x11,
    SubscribeDeny      = 0x12,
    Unsubscribe        = 0x13,
    UnsubscribeAck     = 0x14,
    ConditionsUpdate   = 0x15,
    CapabilitiesQuery  = 0x16,
    CapabilitiesReply  = 0x17,
    Quiesce            = 0x18,
    QuiesceAck         = 0x19,
    Resume             = 0x1A,
    ResumeAck          = 0x1B,
    SidechannelCredit  = 0x1C,
}

impl ChannelKind {
    pub fn all_variants() -> &'static [ChannelKind] {
        &[
            Self::Hello, Self::HelloAck, Self::Goodbye, Self::GoodbyeAck,
            Self::Ping, Self::Pong, Self::Ack, Self::Nack,
            Self::Credit, Self::Backpressure, Self::BackpressureClear,
            Self::RotateInit, Self::RotateCommit, Self::Revoke, Self::ChannelError,
            Self::Subscribe, Self::SubscribeAck, Self::SubscribeDeny,
            Self::Unsubscribe, Self::UnsubscribeAck, Self::ConditionsUpdate,
            Self::CapabilitiesQuery, Self::CapabilitiesReply,
            Self::Quiesce, Self::QuiesceAck, Self::Resume, Self::ResumeAck,
            Self::SidechannelCredit,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownChannelKind(pub u8);

impl TryFrom<u8> for ChannelKind {
    type Error = UnknownChannelKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Hello),
            0x02 => Ok(Self::HelloAck),
            0x03 => Ok(Self::Goodbye),
            0x04 => Ok(Self::GoodbyeAck),
            0x05 => Ok(Self::Ping),
            0x06 => Ok(Self::Pong),
            0x07 => Ok(Self::Ack),
            0x08 => Ok(Self::Nack),
            0x09 => Ok(Self::Credit),
            0x0A => Ok(Self::Backpressure),
            0x0B => Ok(Self::BackpressureClear),
            0x0C => Ok(Self::RotateInit),
            0x0D => Ok(Self::RotateCommit),
            0x0E => Ok(Self::Revoke),
            0x0F => Ok(Self::ChannelError),
            0x10 => Ok(Self::Subscribe),
            0x11 => Ok(Self::SubscribeAck),
            0x12 => Ok(Self::SubscribeDeny),
            0x13 => Ok(Self::Unsubscribe),
            0x14 => Ok(Self::UnsubscribeAck),
            0x15 => Ok(Self::ConditionsUpdate),
            0x16 => Ok(Self::CapabilitiesQuery),
            0x17 => Ok(Self::CapabilitiesReply),
            0x18 => Ok(Self::Quiesce),
            0x19 => Ok(Self::QuiesceAck),
            0x1A => Ok(Self::Resume),
            0x1B => Ok(Self::ResumeAck),
            0x1C => Ok(Self::SidechannelCredit),
            other => Err(UnknownChannelKind(other)),
        }
    }
}

/// Stream (Class S) frame kinds on the Data Lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamKind {
    Open       = 0x01,
    Payload    = 0x02,
    Fault      = 0x03,
    Fin        = 0x04,
    Ack        = 0x05,
    Nack       = 0x06,
    Reset      = 0x07,
    Cancel     = 0x08,
    CancelAck  = 0x09,
    Resume     = 0x0A,
    ResumeDeny = 0x0B,
    Credit     = 0x0C,
    Reference  = 0x0D,
    Sack       = 0x0E,
}

impl StreamKind {
    pub fn all_variants() -> &'static [StreamKind] {
        &[
            Self::Open, Self::Payload, Self::Fault, Self::Fin,
            Self::Ack, Self::Nack, Self::Reset, Self::Cancel,
            Self::CancelAck, Self::Resume, Self::ResumeDeny,
            Self::Credit, Self::Reference, Self::Sack,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownStreamKind(pub u8);

impl TryFrom<u8> for StreamKind {
    type Error = UnknownStreamKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Open),
            0x02 => Ok(Self::Payload),
            0x03 => Ok(Self::Fault),
            0x04 => Ok(Self::Fin),
            0x05 => Ok(Self::Ack),
            0x06 => Ok(Self::Nack),
            0x07 => Ok(Self::Reset),
            0x08 => Ok(Self::Cancel),
            0x09 => Ok(Self::CancelAck),
            0x0A => Ok(Self::Resume),
            0x0B => Ok(Self::ResumeDeny),
            0x0C => Ok(Self::Credit),
            0x0D => Ok(Self::Reference),
            0x0E => Ok(Self::Sack),
            other => Err(UnknownStreamKind(other)),
        }
    }
}

/// Datagram (Class D) frame kinds on the Control Lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DatagramKind {
    Request = 0x01,
    Reply   = 0x02,
    Notify  = 0x03,
    Publish = 0x04,
    Reject  = 0x05,
}

impl DatagramKind {
    pub fn all_variants() -> &'static [DatagramKind] {
        &[Self::Request, Self::Reply, Self::Notify, Self::Publish, Self::Reject]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownDatagramKind(pub u8);

impl TryFrom<u8> for DatagramKind {
    type Error = UnknownDatagramKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Request),
            0x02 => Ok(Self::Reply),
            0x03 => Ok(Self::Notify),
            0x04 => Ok(Self::Publish),
            0x05 => Ok(Self::Reject),
            other => Err(UnknownDatagramKind(other)),
        }
    }
}

/// Audit (Class A) frame kinds on the Audit Lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AuditKind {
    Checkpoint = 0x01,
    Query      = 0x02,
    Proof      = 0x03,
    Gap        = 0x04,
    Replay     = 0x05,
}

impl AuditKind {
    pub fn all_variants() -> &'static [AuditKind] {
        &[Self::Checkpoint, Self::Query, Self::Proof, Self::Gap, Self::Replay]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAuditKind(pub u8);

impl TryFrom<u8> for AuditKind {
    type Error = UnknownAuditKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Checkpoint),
            0x02 => Ok(Self::Query),
            0x03 => Ok(Self::Proof),
            0x04 => Ok(Self::Gap),
            0x05 => Ok(Self::Replay),
            other => Err(UnknownAuditKind(other)),
        }
    }
}

/// Handoff (Class H) frame kinds on the Handoff Lane.
///
/// v4: replaces the per-transfer Offer/Accept/Reject/Revoke/Confirm
/// lifecycle with persistent arena slot coordination and DMA-BUF metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HandoffKind {
    /// SharedMemRef — writer published a slot (writer→reader).
    ArenaWrite   = 0x01,
    /// SlotRelease — reader finished with a slot (reader→writer).
    SlotRelease  = 0x02,
    /// ArenaSetup — server sends arena metadata after fd delivery (server→client).
    ArenaSetup   = 0x03,
    /// ArenaAck — client confirms arena import (client→server).
    ArenaAck     = 0x04,
    /// DmaBufRef — GPU buffer metadata for Tier 3 pass-through (writer→reader).
    DmaBufRef    = 0x05,
}

impl HandoffKind {
    pub fn all_variants() -> &'static [HandoffKind] {
        &[Self::ArenaWrite, Self::SlotRelease, Self::ArenaSetup, Self::ArenaAck, Self::DmaBufRef]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownHandoffKind(pub u8);

impl TryFrom<u8> for HandoffKind {
    type Error = UnknownHandoffKind;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ArenaWrite),
            0x02 => Ok(Self::SlotRelease),
            0x03 => Ok(Self::ArenaSetup),
            0x04 => Ok(Self::ArenaAck),
            0x05 => Ok(Self::DmaBufRef),
            other => Err(UnknownHandoffKind(other)),
        }
    }
}
