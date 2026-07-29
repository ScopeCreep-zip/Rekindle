//! CHANNEL_GOODBYE_ACK handler — completes bidirectional shutdown.

use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SessionStateHandle;
use crate::v4::session::state::SessionEvent;

pub fn handle(
    shared: &SessionStateHandle,
    local_goodbye_sent: bool,
    peer_final_session_seq: Option<u64>,
    _payload: &[u8],
) -> Result<(), HandlerError> {
    if local_goodbye_sent && peer_final_session_seq.is_some() {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::BothGoodbyeAcked)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }
    Ok(())
}
