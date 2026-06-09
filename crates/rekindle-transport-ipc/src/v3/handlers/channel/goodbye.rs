//! CHANNEL_GOODBYE handler — full bidirectional shutdown per §24.2.

use std::time::{Duration, Instant};

use crate::v3::codec::channel::goodbye as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;
use crate::v3::session::state::SessionEvent;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let goodbye = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.session_state_mut()
        .apply(SessionEvent::GoodbyeReceived)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    ctx.set_peer_final_session_seq(goodbye.final_session_seq);

    let drain_ms = if goodbye.drain_timeout_ms > 0 {
        goodbye.drain_timeout_ms as u64
    } else {
        5000
    };
    ctx.set_drain_deadline(Instant::now() + Duration::from_millis(drain_ms));

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::GoodbyeAck,
        payload: codec::encode(&codec::GoodbyePayload {
            reason_code: 0,
            drain_timeout_ms: 0,
            final_session_seq: ctx.send_seq(),
        }),
    });

    if !ctx.local_goodbye_sent() {
        ctx.push_outbound(OutboundFrame::Channel {
            kind: OutboundChannelKind::Goodbye,
            payload: codec::encode(&codec::GoodbyePayload {
                reason_code: 0,
                drain_timeout_ms: drain_ms as u32,
                final_session_seq: ctx.send_seq(),
            }),
        });
        ctx.mark_local_goodbye_sent();
    }

    Ok(())
}
