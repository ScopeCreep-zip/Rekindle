use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::session::state::SessionEvent;

pub fn handle(ctx: &mut SessionContext, _payload: &[u8]) -> Result<(), HandlerError> {
    ctx.session_state_mut()
        .apply(SessionEvent::ChannelErrorReceived)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;
    Ok(())
}
