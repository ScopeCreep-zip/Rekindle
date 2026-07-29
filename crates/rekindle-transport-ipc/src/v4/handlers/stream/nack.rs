//! STREAM_NACK handler — sender-side rejection.
//!
//! Transitions the stream to Failed and notifies the application via
//! router.on_bulk_failed() with the NACK reason and chunk index so
//! the sender's pending_bulk waiter resolves immediately with a
//! diagnostic error instead of hanging until timeout.

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::nack as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::state::StreamEvent;

pub fn handle(
    stream_registry: &mut StreamRegistry,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let nack = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    tracing::warn!(
        stream_id = header.stream_id,
        reason = ?nack.reason_code,
        chunk = nack.rejected_chunk_idx,
        detail = %nack.detail,
        "stream NACK received"
    );

    let current_state = stream_registry.state(
        header.stream_id, crate::v4::stream::registry::Direction::Outbound,
    );
    if current_state.is_some() {
        let _ = stream_registry.transition(
            header.stream_id,
            crate::v4::stream::registry::Direction::Outbound,
            StreamEvent::NackReceived,
        );
    }

    router.on_bulk_failed(
        info, header.stream_id, uuid::Uuid::nil(),
        &format!("NACK: {:?} at chunk {}", nack.reason_code, nack.rejected_chunk_idx),
    );

    Ok(())
}
