//! Pure utility functions — no state, no side effects beyond logging.
//!
//! All functions take explicit parameters — router, info, state.

use crate::v4::io::read_task::SessionOutcome;
use crate::v4::router::{ConnectionInfo, ConnectionPhase, FrameRouter};
use crate::v4::session::state::SessionState;

/// Map a SessionOutcome to the application-visible ConnectionPhase.
pub(crate) fn outcome_to_phase(outcome: &SessionOutcome) -> ConnectionPhase {
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

/// Map a SessionState to the application-visible ConnectionPhase.
pub(crate) fn state_to_phase(state: SessionState) -> ConnectionPhase {
    match state {
        SessionState::Pending | SessionState::Handshaking => ConnectionPhase::Handshaking,
        SessionState::Established | SessionState::Rotating | SessionState::Quiesced => ConnectionPhase::Established,
        SessionState::Draining => ConnectionPhase::Draining,
        SessionState::Closed => ConnectionPhase::Closed,
    }
}

/// Notify the router of a terminal state transition and return the outcome.
/// Takes explicit router, info, and session_state — no god object.
pub(crate) fn terminate(
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    session_state: SessionState,
    outcome: SessionOutcome,
) -> SessionOutcome {
    let old_phase = state_to_phase(session_state);
    let new_phase = outcome_to_phase(&outcome);
    tracing::info!(
        conn_id = info.conn_id,
        session_id = %info.session_id,
        ?old_phase, ?new_phase, ?outcome,
        "terminate: firing on_connection_state_change",
    );
    router.on_connection_state_change(info, old_phase, new_phase);
    outcome
}

/// Notify the router of a non-terminal state transition.
pub(crate) fn notify_state_change(
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    old_phase: ConnectionPhase,
    new_phase: ConnectionPhase,
) {
    tracing::info!(
        conn_id = info.conn_id,
        session_id = %info.session_id,
        ?old_phase, ?new_phase,
        "notify_state_change: firing on_connection_state_change",
    );
    router.on_connection_state_change(info, old_phase, new_phase);
}

/// Generate a random u64 for PING nonces.
pub(crate) fn rand_nonce() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}

/// Wall-clock nanoseconds since Unix epoch.
pub(crate) fn wall_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
