use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    if payload.len() < 16 {
        return Err(HandlerError::CodecFailed("accept payload too short".into()));
    }

    let handoff_id = uuid::Uuid::from_bytes(
        payload[0..16].try_into().map_err(|_| HandlerError::CodecFailed("bad uuid".into()))?
    );

    // On Linux with a coordinator: complete the handoff lifecycle.
    // receive_accept produces HandoffOutcome::Delivered with timing data.
    #[cfg(target_os = "linux")]
    if let Some(coord) = ctx.handoff_coordinator_mut() {
        match coord.receive_accept(handoff_id) {
            Ok(outcome) => {
                tracing::debug!(?outcome, "handoff accept completed via coordinator");
            }
            Err(e) => {
                tracing::warn!(handoff_id = %handoff_id, error = ?e,
                    "coordinator receive_accept failed — cleaning up");
            }
        }
    }

    ctx.remove_pending_handoff(&handoff_id);
    ctx.fallback_tracker_mut().record_success();

    Ok(())
}
