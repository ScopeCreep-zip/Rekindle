//! Pure utility functions — no state, no side effects beyond logging.

use crate::v3::context::SessionContext;
use crate::v3::io::read_task::SessionOutcome;
use crate::v3::router::ConnectionPhase;
use crate::v3::session::state::SessionState;

use super::ControlAction;

/// Map a SessionOutcome to the application-visible ConnectionPhase.
fn outcome_to_phase(outcome: &SessionOutcome) -> ConnectionPhase {
    match outcome {
        SessionOutcome::Closed { .. } => ConnectionPhase::Closed,
        SessionOutcome::HeartbeatTimeout { .. } => ConnectionPhase::Dead,
        SessionOutcome::EnvelopeMacFailed { .. }
        | SessionOutcome::HeaderMacFailed { .. }
        | SessionOutcome::AeadVerificationFailed { .. }
        | SessionOutcome::ReplayDetected { .. }
        | SessionOutcome::WireVersionUnsupported { .. }
        | SessionOutcome::LaneUnknown { .. }
        | SessionOutcome::ReservedBitSet { .. }
        | SessionOutcome::SequenceNonMonotonic { .. }
        | SessionOutcome::FrameTooLarge { .. }
        | SessionOutcome::FrameMalformed { .. }
        | SessionOutcome::ChannelError { .. }
        | SessionOutcome::NonceExhausted => ConnectionPhase::Dead,
        SessionOutcome::AuditChainDivergence { .. } => ConnectionPhase::Dead,
        SessionOutcome::SubstrateReadFailed { .. }
        | SessionOutcome::ConnectionLost => ConnectionPhase::Dead,
        SessionOutcome::QuiescenceTimeout { .. } => ConnectionPhase::Closed,
        SessionOutcome::RotationTimeout => ConnectionPhase::Dead,
        SessionOutcome::DrainTimeout { .. } => ConnectionPhase::Closed,
    }
}

/// Map the current SessionState to the application-visible ConnectionPhase.
fn session_state_to_phase(ctx: &SessionContext) -> ConnectionPhase {
    match ctx.session_state() {
        SessionState::Pending | SessionState::Handshaking => ConnectionPhase::Handshaking,
        SessionState::Established | SessionState::Rotating | SessionState::Quiesced => ConnectionPhase::Established,
        SessionState::Draining => ConnectionPhase::Draining,
        SessionState::Closed => ConnectionPhase::Closed,
    }
}

/// Notify the router of a terminal state transition and return the outcome.
pub(super) fn terminate(ctx: &SessionContext, outcome: SessionOutcome) -> SessionOutcome {
    let info = ctx.connection_info().clone();
    let old_phase = session_state_to_phase(ctx);
    let new_phase = outcome_to_phase(&outcome);
    tracing::info!(
        conn_id = info.conn_id,
        session_id = %info.session_id,
        ?old_phase, ?new_phase, ?outcome,
        "terminate: firing on_connection_state_change",
    );
    ctx.router().on_connection_state_change(&info, old_phase, new_phase);
    outcome
}

/// Notify the router of a non-terminal state transition.
pub(super) fn notify_state_change(ctx: &SessionContext, old_phase: ConnectionPhase, new_phase: ConnectionPhase) {
    let info = ctx.connection_info().clone();
    tracing::info!(
        conn_id = info.conn_id,
        session_id = %info.session_id,
        ?old_phase, ?new_phase,
        "notify_state_change: firing on_connection_state_change",
    );
    ctx.router().on_connection_state_change(&info, old_phase, new_phase);
}

/// Map a ControlAction to a static string for tracing spans.
pub(super) fn action_name(action: &ControlAction) -> &'static str {
    match action {
        ControlAction::WriteFailed(_) => "WriteFailed",
        ControlAction::PongTimeout => "PongTimeout",
        ControlAction::HeartbeatTick => "HeartbeatTick",
        ControlAction::RecvFrame(_) => "RecvFrame",
        ControlAction::ReadFinished(_) => "ReadFinished",
        ControlAction::OutboundAuditLink(_) => "OutboundAuditLink",
        ControlAction::InboundAuditLink(_) => "InboundAuditLink",
        ControlAction::BulkDecrypted(_) => "BulkDecrypted",
        ControlAction::PendingFinReceived(_) => "PendingFinReceived",
        ControlAction::SequencedOutbound(_) => "SequencedOutbound",
        ControlAction::ClientOutbound(_) => "ClientOutbound",
    }
}

/// Generate a random u64 for PING nonces.
pub(super) fn rand_nonce() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}

/// Wall-clock nanoseconds since Unix epoch.
pub(super) fn wall_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
