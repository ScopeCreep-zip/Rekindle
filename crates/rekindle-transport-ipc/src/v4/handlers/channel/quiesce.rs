//! CHANNEL_QUIESCE / CHANNEL_RESUME handlers.

use std::time::{Duration, Instant};

use crate::v4::codec::channel::quiesce as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SessionStateHandle;
use crate::v4::session::state::SessionEvent;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle_quiesce(
    shared: &SessionStateHandle,
    outbound: &mut Vec<OutboundFrame>,
    quiescence_deadline: &mut Option<Instant>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let quiesce = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::QuiesceAcked)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }

    *quiescence_deadline = Some(Instant::now() + Duration::from_millis(quiesce.duration_ms));

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::QuiesceAck,
        payload: codec::encode(&codec::QuiescePayload {
            duration_ms: quiesce.duration_ms,
            reason_code: 0,
        }),
    });

    Ok(())
}

pub fn handle_resume(
    shared: &SessionStateHandle,
    outbound: &mut Vec<OutboundFrame>,
    quiescence_deadline: &mut Option<Instant>,
    _payload: &[u8],
) -> Result<(), HandlerError> {
    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::ResumeAcked)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }

    *quiescence_deadline = None;

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::ResumeAck,
        payload: codec::encode(&codec::QuiescePayload {
            duration_ms: 0,
            reason_code: 0,
        }),
    });

    Ok(())
}
