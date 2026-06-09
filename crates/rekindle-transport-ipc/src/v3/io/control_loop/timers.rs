//! Deadline enforcement — drain, quiescence, and rotation timeouts.

use std::time::Instant;

use crate::v3::context::SessionContext;
use crate::v3::io::read_task::SessionOutcome;
use crate::v3::session::state::SessionEvent;

use super::util;

/// Check all active deadlines. Returns Some(SessionOutcome) if any
/// deadline has been exceeded — the control loop must return immediately.
pub(super) fn check_deadlines(ctx: &mut SessionContext) -> Option<SessionOutcome> {
    let now = Instant::now();

    if let Some(deadline) = ctx.drain_deadline() {
        if now >= deadline {
            let _ = ctx.session_state_mut().apply(SessionEvent::DrainTimeout);
            return Some(util::terminate(ctx, SessionOutcome::DrainTimeout {
                local_goodbye_sent: ctx.local_goodbye_sent(),
                peer_goodbye_received: ctx.peer_final_session_seq().is_some(),
                active_streams: ctx.stream_registry().active_count(),
            }));
        }
    }

    if let Some(deadline) = ctx.quiescence_deadline() {
        if now >= deadline {
            let _ = ctx.session_state_mut().apply(SessionEvent::QuiescenceTimeout);
            return Some(util::terminate(ctx, SessionOutcome::QuiescenceTimeout { duration_ms: 0 }));
        }
    }

    if let Some(deadline) = ctx.rotation_deadline() {
        if now >= deadline {
            let _ = ctx.session_state_mut().apply(SessionEvent::RotationTimeout);
            return Some(util::terminate(ctx, SessionOutcome::RotationTimeout));
        }
    }

    // Epoch key retirement is slot overwrite — no timer needed.
    // Old keys are dropped when the next rotation installs into the same slot.

    None
}
