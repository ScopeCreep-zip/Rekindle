//! CHANNEL_GOODBYE_ACK handler — completes bidirectional shutdown.
//!
//! When we receive GOODBYE_ACK, the peer acknowledged our GOODBYE.
//! If we also received the peer's GOODBYE (peer_final_session_seq is set),
//! both sides have exchanged GOODBYE+GOODBYE_ACK → BothGoodbyeAcked → Closed.

use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::session::state::SessionEvent;

pub fn handle(ctx: &mut SessionContext, _payload: &[u8]) -> Result<(), HandlerError> {
    if ctx.local_goodbye_sent() && ctx.peer_final_session_seq().is_some() {
        ctx.session_state_mut()
            .apply(SessionEvent::BothGoodbyeAcked)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }
    Ok(())
}
