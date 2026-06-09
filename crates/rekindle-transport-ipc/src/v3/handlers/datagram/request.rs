use crate::v3::codec::channel::ack as ack_codec;
use crate::v3::codec::datagram::request as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let req = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    // Rate limit: prevent a malicious client from sending unlimited
    // concurrent requests that exhaust server memory.
    if ctx.pending_request_count() >= ctx.max_pending_requests() {
        return Err(HandlerError::PendingRequestsExhausted);
    }

    // Track the inbound request for rate limiting.
    ctx.register_pending_request(req.message_id, req.reply_timeout_ms);

    // Deliver to the application router.
    let info = ctx.connection_info().clone();
    ctx.router().on_request(
        &info,
        req.message_id,
        req.sender_clearance,
        &req.application_payload,
    );

    // Transport-level ACK — request received and dispatched.
    // The pending slot stays held until the application sends
    // DATAGRAM_REPLY with this message_id as correlation_id.
    // The control loop's drain_outbound resolves the slot when
    // it encodes the reply. This bounds concurrent in-flight
    // application work at max_pending_requests.
    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(req.message_id),
    });

    Ok(())
}
