//! Pure utility functions — no state, no side effects beyond logging.

use crate::v3::context::SessionContext;
use crate::v3::io::read_task::SessionOutcome;

use super::ControlAction;

/// Notify the router of a state transition and return the outcome.
pub(super) fn terminate(ctx: &SessionContext, outcome: SessionOutcome) -> SessionOutcome {
    let info = ctx.connection_info().clone();
    let old_state = format!("{:?}", ctx.session_state());
    let new_state = format!("{outcome:?}");
    ctx.router().on_connection_state_change(&info, &old_state, &new_state);
    outcome
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
