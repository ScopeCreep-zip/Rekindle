//! CHANNEL_ERROR handler — transitions session to terminal state.

use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SessionStateHandle;
use crate::v4::session::state::SessionEvent;

pub fn handle(shared: &SessionStateHandle, _payload: &[u8]) -> Result<(), HandlerError> {
    let mut s = shared.write();
    s.session_state
        .apply(SessionEvent::ChannelErrorReceived)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;
    Ok(())
}
