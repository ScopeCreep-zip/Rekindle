//! Session state machine — every valid transition is explicit.

use crate::v3::wire::frame_class::FrameClass;
use crate::v3::wire::frame_kind::{ChannelKind, AuditKind, StreamKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Pending,
    Handshaking,
    Established,
    Rotating,
    Quiesced,
    Draining,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    SubstrateConnected,
    HelloAckValid,
    GoodbyeSent,
    GoodbyeReceived,
    RotateInitSent,
    RotateInitReceived,
    RotateCommitSent,
    RotateCommitValid,
    RotationTimeout,
    QuiesceAcked,
    QuiescenceTimeout,
    ResumeAcked,
    HeartbeatTimeout,
    AeadFailed,
    EmacFailed,
    HeaderMacFailed,
    SubstrateEof,
    SubstrateError,
    ChannelErrorReceived,
    ReplayDetected,
    AuditChainDivergence,
    FrameClassUnknown,
    LaneUnknown,
    ReservedBitSet,
    BothGoodbyeAcked,
    DrainTimeout,
    HandshakeTimeout,
    HandshakeNoiseFailed,
    PeerUnregistered,
    CapabilityMismatch,
    StreamFrameReceived,
}

impl SessionEvent {
    pub fn all_variants() -> &'static [SessionEvent] {
        &[
            Self::SubstrateConnected, Self::HelloAckValid,
            Self::GoodbyeSent, Self::GoodbyeReceived,
            Self::RotateInitSent, Self::RotateInitReceived,
            Self::RotateCommitSent, Self::RotateCommitValid, Self::RotationTimeout,
            Self::QuiesceAcked, Self::QuiescenceTimeout, Self::ResumeAcked,
            Self::HeartbeatTimeout, Self::AeadFailed, Self::EmacFailed,
            Self::HeaderMacFailed, Self::SubstrateEof, Self::SubstrateError,
            Self::ChannelErrorReceived, Self::ReplayDetected,
            Self::AuditChainDivergence, Self::FrameClassUnknown,
            Self::LaneUnknown, Self::ReservedBitSet,
            Self::BothGoodbyeAcked, Self::DrainTimeout,
            Self::HandshakeTimeout, Self::HandshakeNoiseFailed,
            Self::PeerUnregistered, Self::CapabilityMismatch,
            Self::StreamFrameReceived,
        ]
    }
}

#[derive(Debug)]
pub struct SessionTransitionError {
    pub from: SessionState,
    pub event: SessionEvent,
}

impl std::fmt::Display for SessionTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid transition from {:?} on {:?}", self.from, self.event)
    }
}

impl SessionState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Apply an event, transitioning the state if valid.
    /// On error, the state is not modified.
    pub fn apply(&mut self, event: SessionEvent) -> Result<(), SessionTransitionError> {
        let from = *self;
        let next = self.next_state(event)?;
        tracing::info!(
            from = ?from,
            event = ?event,
            to = ?next,
            "session state transition"
        );
        *self = next;
        Ok(())
    }

    fn next_state(self, event: SessionEvent) -> Result<SessionState, SessionTransitionError> {
        use SessionEvent as E;
        use SessionState as S;

        let err = || SessionTransitionError { from: self, event };

        match (self, event) {
            (S::Pending, E::SubstrateConnected) => Ok(S::Handshaking),

            (S::Handshaking, E::HelloAckValid)
            | (S::Rotating, E::RotateCommitSent)     // responder exits Rotating after sending COMMIT
            | (S::Rotating, E::RotateCommitValid)     // initiator exits Rotating after receiving COMMIT
            | (S::Quiesced, E::ResumeAcked) => Ok(S::Established),

            (S::Established, E::GoodbyeSent | E::GoodbyeReceived) => Ok(S::Draining),
            // Bidirectional shutdown: we sent GOODBYE (→ Draining), then
            // receive the peer's GOODBYE. Stay in Draining — both sides
            // have now sent GOODBYE, awaiting mutual GOODBYE_ACK.
            (S::Draining, E::GoodbyeReceived) => Ok(S::Draining),
            (S::Established, E::RotateInitSent | E::RotateInitReceived) => Ok(S::Rotating),
            (S::Established, E::QuiesceAcked) => Ok(S::Quiesced),

            (S::Handshaking, E::HandshakeTimeout | E::HandshakeNoiseFailed
                | E::PeerUnregistered | E::CapabilityMismatch
                | E::SubstrateError | E::SubstrateEof)
            | (S::Established, E::HeartbeatTimeout | E::AeadFailed | E::EmacFailed
                | E::HeaderMacFailed | E::SubstrateEof | E::SubstrateError
                | E::ChannelErrorReceived | E::ReplayDetected | E::AuditChainDivergence
                | E::FrameClassUnknown | E::LaneUnknown | E::ReservedBitSet)
            | (S::Rotating, E::RotationTimeout)
            | (S::Quiesced, E::QuiescenceTimeout | E::SubstrateEof | E::SubstrateError)
            | (S::Draining, E::BothGoodbyeAcked | E::DrainTimeout) => Ok(S::Closed),

            _ => Err(err()),
        }
    }
}

/// Check whether a specific (FrameClass, FrameKind) pair is allowed
/// in the given SessionState.
pub fn is_frame_allowed(state: SessionState, class: FrameClass, kind: u8) -> bool {
    use SessionState as S;
    match state {
        S::Pending | S::Closed => false,

        S::Handshaking => {
            class == FrameClass::Channel
                && (kind == ChannelKind::Hello as u8 || kind == ChannelKind::HelloAck as u8)
        }

        S::Established => true,

        // Rotating is a transient state that lasts only for the duration
        // of the rotation handshake (microseconds on the initiator,
        // instant on the responder via RotateCommitSent). In-flight frames
        // from before rotation started (replies, bulk chunks, audit frames)
        // must not be rejected. The only frames that are truly invalid
        // during rotation are new stream opens (which would create state
        // that spans the key boundary) and lifecycle changes (goodbye,
        // quiesce) that conflict with the rotation protocol.
        S::Rotating => match class {
            FrameClass::Channel => {
                kind != ChannelKind::Goodbye as u8
                    && kind != ChannelKind::GoodbyeAck as u8
                    && kind != ChannelKind::Hello as u8
                    && kind != ChannelKind::HelloAck as u8
                    && kind != ChannelKind::Quiesce as u8
                    && kind != ChannelKind::QuiesceAck as u8
                    && kind != ChannelKind::Resume as u8
                    && kind != ChannelKind::ResumeAck as u8
            }
            // Stream: allow in-flight completions (payload, fin, ack, etc.)
            // but reject new stream opens — a new stream's lifecycle would
            // span the key boundary.
            FrameClass::Stream => kind != StreamKind::Open as u8,
            // Datagram, Audit, Handoff — all may have been dispatched
            // before rotation started. Allow to complete.
            FrameClass::Datagram | FrameClass::Audit | FrameClass::Handoff => true,
        }

        S::Quiesced => {
            class == FrameClass::Channel
                && (kind == ChannelKind::Ping as u8
                    || kind == ChannelKind::Pong as u8
                    || kind == ChannelKind::Resume as u8
                    || kind == ChannelKind::ResumeAck as u8)
        }

        S::Draining => match class {
            FrameClass::Channel => {
                kind == ChannelKind::Ping as u8
                    || kind == ChannelKind::Pong as u8
                    || kind == ChannelKind::Ack as u8
                    || kind == ChannelKind::Goodbye as u8
                    || kind == ChannelKind::GoodbyeAck as u8
            }
            FrameClass::Audit => kind == AuditKind::Checkpoint as u8,
            _ => false,
        },
    }
}
