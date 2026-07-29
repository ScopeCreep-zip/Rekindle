//! STREAM_RESET handler.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::handlers::HandlerError;
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::state::StreamEvent;

pub fn handle(
    stream_registry: &mut StreamRegistry,
    reassemblers: &mut HashMap<u8, Reassembler>,
    header: &StreamHeaderInfo,
    _payload: &[u8],
) -> Result<(), HandlerError> {
    stream_registry.transition(
        header.stream_id,
        crate::v4::stream::registry::Direction::Inbound,
        StreamEvent::ResetReceived,
    ).map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    reassemblers.remove(&header.stream_id);

    stream_registry.transition(
        header.stream_id,
        crate::v4::stream::registry::Direction::Inbound,
        StreamEvent::CleanupComplete,
    ).map_err(|_| HandlerError::StreamNotFound(header.stream_id))?;

    Ok(())
}
