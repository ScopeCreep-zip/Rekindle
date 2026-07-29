//! Outbound frame types — the interface between handlers and the encoder.
//!
//! `OutboundFrame` is the typed representation of a frame queued for
//! outbound delivery. Handlers push these; the lane task's drain loop
//! encodes and routes them to the write task.
//!
//! `SequencedOutbound` wraps an `OutboundFrame` with confirmation and
//! completion oneshots for operations that need send ordering guarantees
//! (e.g., STREAM_OPEN must reach the write task before STREAM_PAYLOAD).

use crate::v4::wire::frame_kind::{
    AuditKind, ChannelKind, DatagramKind, HandoffKind, StreamKind,
};
use crate::v4::wire::frame_class::FrameClass;
use crate::v4::wire::lane::Lane;

// Re-export wire types that handlers use for OutboundFrame construction.
// Handlers import these from here — they don't reach into wire::frame_kind directly.
pub use crate::v4::wire::frame_kind::StreamKind as OutboundStreamKind;
pub use crate::v4::wire::frame_kind::ChannelKind as OutboundChannelKind;
pub use crate::v4::wire::frame_kind::DatagramKind as OutboundDatagramKind;
pub use crate::v4::wire::frame_kind::AuditKind as OutboundAuditKind;
pub use crate::v4::wire::frame_kind::HandoffKind as OutboundHandoffKind;
pub use crate::v4::wire::failure::FailureCode as OutboundFailureCode;

/// A frame queued for outbound delivery. Handlers push these;
/// the lane task encodes and writes them.
///
/// Typed enums for kind fields — handlers never use raw u8 wire values.
/// The encoder converts to u8 when serializing to wire bytes.
#[derive(Debug)]
pub enum OutboundFrame {
    Channel { kind: ChannelKind, payload: Vec<u8> },
    Datagram { kind: DatagramKind, payload: Vec<u8> },
    Data { stream_id: u8, kind: StreamKind, chunk_index: u32, payload: Vec<u8> },
    Audit { kind: AuditKind, payload: Vec<u8> },
    Handoff { kind: HandoffKind, payload: Vec<u8> },
}

impl OutboundFrame {
    /// Which wire lane this frame belongs to.
    pub fn lane(&self) -> Lane {
        match self {
            Self::Channel { .. } | Self::Datagram { .. } => Lane::Control,
            Self::Data { .. } => Lane::Data,
            Self::Audit { .. } => Lane::Audit,
            Self::Handoff { .. } => Lane::Handoff,
        }
    }

    /// The FrameClass byte for the encoder.
    pub fn class_byte(&self) -> u8 {
        match self {
            Self::Channel { .. } => FrameClass::Channel as u8,
            Self::Datagram { .. } => FrameClass::Datagram as u8,
            Self::Data { .. } => FrameClass::Stream as u8,
            Self::Audit { .. } => FrameClass::Audit as u8,
            Self::Handoff { .. } => FrameClass::Handoff as u8,
        }
    }

    /// The FrameKind byte for the encoder.
    pub fn kind_byte(&self) -> u8 {
        match self {
            Self::Channel { kind, .. } => *kind as u8,
            Self::Datagram { kind, .. } => *kind as u8,
            Self::Data { kind, .. } => *kind as u8,
            Self::Audit { kind, .. } => *kind as u8,
            Self::Handoff { kind, .. } => *kind as u8,
        }
    }

    /// The payload bytes.
    pub fn payload_bytes(&self) -> &[u8] {
        match self {
            Self::Channel { payload, .. }
            | Self::Datagram { payload, .. }
            | Self::Data { payload, .. }
            | Self::Audit { payload, .. }
            | Self::Handoff { payload, .. } => payload,
        }
    }

    /// Whether this is a Data lane frame (needs stream header encoding).
    pub fn is_data_frame(&self) -> bool {
        matches!(self, Self::Data { .. })
    }

    /// Construct a CHANNEL_ERROR outbound frame from an error code and message.
    pub fn channel_error(error_code: u16, message: &str) -> Self {
        use crate::v4::codec::channel::error as error_codec;
        let payload = error_codec::encode(&error_codec::ErrorPayload {
            error_code,
            message: message.to_owned(),
        });
        Self::Channel { kind: ChannelKind::ChannelError, payload }
    }
}

/// A sequenced outbound frame with send confirmation.
///
/// Used by BulkSender: STREAM_OPEN must reach the write task before
/// STREAM_PAYLOAD chunks are dispatched to rayon. The oneshot resolves
/// after the frame is encoded and sent to the lane channel.
pub struct SequencedOutbound {
    pub frame: OutboundFrame,
    /// Resolved after the frame is encoded and sent to the write task.
    pub confirm: tokio::sync::oneshot::Sender<()>,
    /// Optional second-phase completion for two-phase operations.
    /// Used by key rotation (resolved after ROTATE_COMMIT is processed).
    pub completion: Option<tokio::sync::oneshot::Sender<Result<(), crate::v4::session::rotation::RotationError>>>,
}
