//! Layer 4: Handlers — one function per FrameKind.
//!
//! Each handler receives `&mut SessionContext` and the decoded payload.
//! Handlers call into subsystems, push `OutboundFrame` entries, and
//! return `Result<(), HandlerError>`.

pub mod channel;
pub mod stream;
pub mod datagram;
pub mod audit;
pub mod handoff;

use crate::v3::context::OutboundFailureCode;

/// Errors returned by handlers. The dispatch layer maps these to
/// NACK/ERROR frames and pushes them to the outbound queue.
#[derive(Debug)]
pub enum HandlerError {
    FrameDisallowedInState,
    ClearanceInsufficient,
    StreamAlreadyOpen(u8),
    StreamIdExhausted,
    StreamNotFound(u8),
    TransferIdUnknown(uuid::Uuid),
    ResumeWindowExpired,
    AuditAnchorMismatch,
    ContentAnchorMismatch,
    ContentHashMismatch,
    DedupCacheMiss,
    DedupClearanceInsufficient,
    PendingRequestsExhausted,
    SubscriptionQuotaExhausted,
    BackpressureExhausted,
    CreditExhausted,
    NonceMismatch,
    NoPendingPing,
    CodecFailed(String),
}

impl HandlerError {
    /// Map to the wire-level failure code for CHANNEL_NACK / CHANNEL_ERROR.
    pub fn failure_code(&self) -> OutboundFailureCode {
        match self {
            Self::FrameDisallowedInState => OutboundFailureCode::FrameDisallowedInState,
            Self::ClearanceInsufficient => OutboundFailureCode::ClearanceInsufficient,
            Self::StreamAlreadyOpen(_) => OutboundFailureCode::StreamAlreadyOpen,
            Self::StreamIdExhausted => OutboundFailureCode::StreamIdExhausted,
            Self::StreamNotFound(_) => OutboundFailureCode::FrameDisallowedInState,
            Self::TransferIdUnknown(_) => OutboundFailureCode::TransferIdUnknown,
            Self::ResumeWindowExpired => OutboundFailureCode::ResumeWindowExpired,
            Self::AuditAnchorMismatch => OutboundFailureCode::AuditAnchorMismatch,
            Self::ContentAnchorMismatch => OutboundFailureCode::ContentAnchorMismatch,
            Self::ContentHashMismatch => OutboundFailureCode::ContentHashMismatch,
            Self::DedupCacheMiss => OutboundFailureCode::DedupCacheMiss,
            Self::DedupClearanceInsufficient => OutboundFailureCode::DedupClearanceInsufficient,
            Self::PendingRequestsExhausted => OutboundFailureCode::PendingRequestsExhausted,
            Self::SubscriptionQuotaExhausted => OutboundFailureCode::SubscriptionQuotaExhausted,
            Self::BackpressureExhausted => OutboundFailureCode::BackpressureExhausted,
            Self::CreditExhausted => OutboundFailureCode::BudgetExhausted,
            Self::NonceMismatch => OutboundFailureCode::ReplayDetected,
            Self::NoPendingPing => OutboundFailureCode::HeartbeatTimeout,
            Self::CodecFailed(_) => OutboundFailureCode::FrameMalformed,
        }
    }
}
