use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::ack as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::registry::Direction;
use crate::v3::stream::state::StreamEvent;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let ack = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let pre_state = ctx.stream_registry().state(header.stream_id, Direction::Outbound);
    tracing::info!(
        stream_id = header.stream_id,
        pre_state = ?pre_state,
        ack_byte_count = ack.ack_byte_count,
        ack_chunk_count = ack.ack_chunk_count,
        "STREAM_ACK handler: transitioning AckForFinReceived"
    );

    match ctx.stream_registry_mut()
        .transition(header.stream_id, Direction::Outbound, StreamEvent::AckForFinReceived)
    {
        Ok(()) => {
            let post_state = ctx.stream_registry().state(header.stream_id, Direction::Outbound);
            tracing::info!(
                stream_id = header.stream_id,
                post_state = ?post_state,
                "STREAM_ACK handler: transition complete, removing reassembler"
            );
        }
        Err(e) => {
            // Stale ACK for a stream that was already closed (force-reopened
            // for a new transfer, or ACK arrived after close). Log and
            // deliver the completion to the application — not fatal.
            tracing::debug!(
                stream_id = header.stream_id,
                pre_state = ?pre_state,
                error = %e,
                "STREAM_ACK handler: transition failed (stale ACK) — delivering completion anyway"
            );
        }
    }

    ctx.remove_reassembler(header.stream_id);

    // Deliver to the application — the client's ClientRouter resolves
    // pending_bulk waiters in on_bulk_complete.
    let info = ctx.connection_info().clone();
    tracing::info!(
        stream_id = header.stream_id,
        transfer_id = %ack.transfer_id,
        ack_byte_count = ack.ack_byte_count,
        ack_chunk_count = ack.ack_chunk_count,
        "STREAM_ACK handler: calling router.on_bulk_complete"
    );
    // No push_bulk_completion here. STREAM_ACK is received by the SENDER
    // (the side that initiated the transfer). The sender's completion is
    // resolved directly by on_bulk_complete → pending_bulk waiter. Pushing
    // a completion through the bridge task would fill bulk_chunk_tx with
    // sentinels nobody reads (recv_bulk is for the RECEIVING side), causing
    // backpressure deadlock after capacity-many sends.
    ctx.router().on_bulk_complete(
        &info,
        header.stream_id,
        ack.transfer_id,
        ack.ack_byte_count,
        ack.ack_chunk_count,
    );

    Ok(())
}
