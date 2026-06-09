use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    if payload.len() < 24 {
        return Err(HandlerError::CodecFailed("confirm payload too short".into()));
    }

    let handoff_id = uuid::Uuid::from_bytes(
        payload[8..24].try_into().map_err(|_| HandlerError::CodecFailed("bad uuid".into()))?
    );

    // Confirm is the final ack — sender has released its memfd end.
    #[cfg(target_os = "linux")]
    if let Some(coord) = ctx.handoff_coordinator_mut() {
        coord.cancel(handoff_id); // cleanup any remaining state
    }

    ctx.remove_pending_handoff(&handoff_id);

    Ok(())
}
