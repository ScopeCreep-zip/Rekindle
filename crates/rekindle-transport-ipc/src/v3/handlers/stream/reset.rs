use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::registry::Direction;
use crate::v3::stream::state::StreamEvent;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, _payload: &[u8]) -> Result<(), HandlerError> {
    ctx.stream_registry_mut()
        .transition(header.stream_id, Direction::Inbound, StreamEvent::ResetReceived)
        .map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    ctx.remove_reassembler(header.stream_id);

    ctx.stream_registry_mut()
        .transition(header.stream_id, Direction::Inbound, StreamEvent::CleanupComplete)
        .map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    Ok(())
}
