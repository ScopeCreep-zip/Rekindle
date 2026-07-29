//! DATAGRAM_REPLY handler.

use crate::v4::codec::datagram::reply as codec;
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};

pub fn handle(
    pending_requests: &mut PendingRequestTracker,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let reply = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    pending_requests.resolve(&reply.correlation_id);

    router.on_reply(info, reply.message_id, reply.correlation_id, reply.status_phase, &reply.application_payload);

    Ok(())
}
