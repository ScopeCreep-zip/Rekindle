//! CHANNEL_GOODBYE handler — full bidirectional shutdown per §24.2.

use std::time::{Duration, Instant};

use crate::v4::codec::channel::goodbye as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SessionStateHandle;
use crate::v4::session::state::SessionEvent;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle(
    shared: &SessionStateHandle,
    outbound: &mut Vec<OutboundFrame>,
    peer_final_session_seq: &mut Option<u64>,
    local_goodbye_sent: &mut bool,
    drain_deadline: &mut Option<Instant>,
    send_seq: u64,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let goodbye = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    tracing::info!(
        peer_final_session_seq = goodbye.final_session_seq,
        drain_timeout_ms = goodbye.drain_timeout_ms,
        reason_code = goodbye.reason_code,
        local_goodbye_already_sent = *local_goodbye_sent,
        send_seq,
        "GOODBYE handler: received CHANNEL_GOODBYE"
    );

    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::GoodbyeReceived)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
        s.shutting_down = true;
    }

    *peer_final_session_seq = Some(goodbye.final_session_seq);

    let drain_ms = if goodbye.drain_timeout_ms > 0 {
        goodbye.drain_timeout_ms as u64
    } else {
        5000
    };
    *drain_deadline = Some(Instant::now() + Duration::from_millis(drain_ms));

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::GoodbyeAck,
        payload: codec::encode(&codec::GoodbyePayload {
            reason_code: 0,
            drain_timeout_ms: 0,
            final_session_seq: send_seq,
        }),
    });

    if !*local_goodbye_sent {
        outbound.push(OutboundFrame::Channel {
            kind: OutboundChannelKind::Goodbye,
            payload: codec::encode(&codec::GoodbyePayload {
                reason_code: 0,
                drain_timeout_ms: drain_ms as u32,
                final_session_seq: send_seq,
            }),
        });
        *local_goodbye_sent = true;
    }

    Ok(())
}
