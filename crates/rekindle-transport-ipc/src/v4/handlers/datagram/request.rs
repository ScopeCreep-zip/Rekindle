//! DATAGRAM_REQUEST handler — rate-limited inbound request delivery.

use crate::v4::codec::channel::ack as ack_codec;
use crate::v4::codec::datagram::request as codec;
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle(
    pending_requests: &mut PendingRequestTracker,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let req = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if pending_requests.is_full() {
        return Err(HandlerError::PendingRequestsExhausted);
    }

    pending_requests.register(req.message_id, req.reply_timeout_ms);

    router.on_request(info, req.message_id, req.sender_clearance, &req.application_payload);

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(req.message_id),
    });

    Ok(())
}
