//! STREAM_ACK handler — sender-side transfer completion.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::ack as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::state::StreamEvent;

pub fn handle(
    stream_registry: &mut StreamRegistry,
    reassemblers: &mut HashMap<u8, Reassembler>,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let ack = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    tracing::info!(
        stream_id = header.stream_id,
        transfer_id = %ack.transfer_id,
        ack_byte_count = ack.ack_byte_count,
        ack_chunk_count = ack.ack_chunk_count,
        "ACK handler: received STREAM_ACK"
    );

    let _ = stream_registry.transition(
        header.stream_id,
        crate::v4::stream::registry::Direction::Outbound,
        StreamEvent::AckForFinReceived,
    );

    reassemblers.remove(&header.stream_id);

    router.on_bulk_complete(
        info, header.stream_id, ack.transfer_id,
        ack.ack_byte_count, ack.ack_chunk_count,
    );

    Ok(())
}
