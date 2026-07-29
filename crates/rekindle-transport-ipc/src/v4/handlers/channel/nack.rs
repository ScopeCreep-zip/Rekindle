//! CHANNEL_NACK handler — resolve rejected pending request.

use crate::v4::codec::channel::nack as codec;
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::HandlerError;

pub fn handle(
    pending_requests: &mut PendingRequestTracker,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let nack = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;
    pending_requests.resolve(&nack.rejected_message_id);
    Ok(())
}
