//! DATAGRAM_REJECT handler.

use crate::v4::codec::datagram::reject as codec;
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};

pub fn handle(
    pending_requests: &mut PendingRequestTracker,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let reject = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    pending_requests.resolve(&reject.rejected_message_id);

    router.on_reject(info, reject.rejected_message_id, reject.reason_code as u32, &reject.detail);

    Ok(())
}
