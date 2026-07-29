//! CHANNEL_ACK handler — resolve pending requests, notify router.

use crate::v4::codec::channel::ack as codec;
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};

pub fn handle(
    pending_requests: &mut PendingRequestTracker,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let ack = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    for message_id in &ack.message_ids {
        pending_requests.resolve(message_id);
    }

    router.on_ack(info, &ack.message_ids);
    Ok(())
}
