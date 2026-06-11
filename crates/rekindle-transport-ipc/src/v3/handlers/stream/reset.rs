use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::state::StreamEvent;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, _payload: &[u8]) -> Result<(), HandlerError> {
    ctx.transition_inbound_stream(header.stream_id, StreamEvent::ResetReceived)
        .map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    ctx.remove_reassembler(header.stream_id);

    ctx.transition_inbound_stream(header.stream_id, StreamEvent::CleanupComplete)
        .map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    Ok(())
}
