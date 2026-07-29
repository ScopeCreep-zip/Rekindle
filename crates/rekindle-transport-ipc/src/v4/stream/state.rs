//! Per-stream state machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    Opening,
    Open,
    Closing,
    Resetting,
    Suspended,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent {
    OpenSent,
    FirstAckReceived,
    ImplicitAckWindowExpired,
    NackReceived,
    PayloadReceived,
    FinSent,
    AckForFinReceived,
    ResetSent,
    ResetReceived,
    CleanupComplete,
    SubstrateFailure,
    ResumeAccepted,
    ResumeDenied,
    WindowExpired,
    CancelSent,
    CancelAckReceived,
}

impl StreamEvent {
    pub fn all_variants() -> &'static [StreamEvent] {
        &[
            Self::OpenSent, Self::FirstAckReceived, Self::ImplicitAckWindowExpired,
            Self::NackReceived, Self::PayloadReceived, Self::FinSent,
            Self::AckForFinReceived, Self::ResetSent, Self::ResetReceived,
            Self::CleanupComplete, Self::SubstrateFailure, Self::ResumeAccepted,
            Self::ResumeDenied, Self::WindowExpired, Self::CancelSent,
            Self::CancelAckReceived,
        ]
    }
}

#[derive(Debug)]
pub struct StreamTransitionError {
    pub from: StreamState,
    pub event: StreamEvent,
}

impl std::fmt::Display for StreamTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid stream transition from {:?} on {:?}", self.from, self.event)
    }
}

impl StreamState {
    pub fn apply(&mut self, event: StreamEvent) -> Result<(), StreamTransitionError> {
        let next = self.next_state(event)?;
        *self = next;
        Ok(())
    }

    fn next_state(self, event: StreamEvent) -> Result<StreamState, StreamTransitionError> {
        use StreamEvent as E;
        use StreamState as S;
        let err = || StreamTransitionError { from: self, event };

        match (self, event) {
            (S::Idle, E::OpenSent) => Ok(S::Opening),

            (S::Opening, E::FirstAckReceived | E::ImplicitAckWindowExpired)
            | (S::Open, E::PayloadReceived)
            | (S::Suspended, E::ResumeAccepted) => Ok(S::Open),

            (S::Opening, E::NackReceived)
            | (S::Suspended, E::ResumeDenied | E::WindowExpired) => Ok(S::Failed),

            (S::Open, E::FinSent | E::CancelSent) => Ok(S::Closing),
            (S::Open, E::ResetSent | E::ResetReceived) => Ok(S::Resetting),
            (S::Open, E::SubstrateFailure) => Ok(S::Suspended),

            (S::Open, E::AckForFinReceived)
            | (S::Closing, E::AckForFinReceived | E::CancelAckReceived)
            | (S::Resetting, E::CleanupComplete) => Ok(S::Closed),

            _ => Err(err()),
        }
    }
}
