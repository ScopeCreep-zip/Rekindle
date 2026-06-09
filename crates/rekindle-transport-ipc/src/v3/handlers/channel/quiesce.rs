use std::time::{Duration, Instant};

use crate::v3::codec::channel::quiesce as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;
use crate::v3::session::state::SessionEvent;

pub fn handle_quiesce(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let quiesce = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.session_state_mut()
        .apply(SessionEvent::QuiesceAcked)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    ctx.set_quiescence_deadline(
        Instant::now() + Duration::from_millis(quiesce.duration_ms)
    );

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::QuiesceAck,
        payload: codec::encode(&codec::QuiescePayload {
            duration_ms: quiesce.duration_ms,
            reason_code: 0,
        }),
    });

    Ok(())
}

pub fn handle_resume(ctx: &mut SessionContext, _payload: &[u8]) -> Result<(), HandlerError> {
    ctx.session_state_mut()
        .apply(SessionEvent::ResumeAcked)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    ctx.clear_quiescence_deadline();

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::ResumeAck,
        payload: codec::encode(&codec::QuiescePayload {
            duration_ms: 0,
            reason_code: 0,
        }),
    });

    Ok(())
}
