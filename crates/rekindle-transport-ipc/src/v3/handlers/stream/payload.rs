use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::ack as ack_codec;
use crate::v3::context::{OutboundFrame, OutboundStreamKind, SessionContext};
use crate::v3::handlers::HandlerError;
use crate::v3::io::lane_channels::PlaintextBuf;
pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let digest = *blake3::hash(payload).as_bytes();

    let reassembler = ctx.reassembler_mut(header.stream_id)
        .ok_or(HandlerError::StreamNotFound(header.stream_id))?;

    let delivered = reassembler.insert_with_digest(
        header.chunk_index, PlaintextBuf::Owned(payload.to_vec()), digest,
    );

    for (chunk_index, chunk_data) in delivered {
        ctx.push_bulk_delivery(header.stream_id, chunk_index, chunk_data);
    }

    // FIN_FOLLOWS: this is the last (only) chunk. Verify content hash
    // and emit STREAM_ACK immediately — no separate FIN frame expected.
    // PendingFinVerify was stored at STREAM_OPEN time for single-chunk transfers.
    if header.fin_follows() {
        if let Some(pending) = ctx.take_pending_fin_verify(header.stream_id) {
            let hash_ok = ctx.reassembler_mut(header.stream_id)
                .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
                .unwrap_or(false);

            if !hash_ok {
                return Err(HandlerError::ContentHashMismatch);
            }

            let total_bytes = ctx.reassembler_mut(header.stream_id)
                .map(|r| r.total_bytes()).unwrap_or(0);
            let total_chunks = ctx.reassembler_mut(header.stream_id)
                .map(|r| r.next_expected()).unwrap_or(0);

            let ack = ack_codec::StreamAckPayload {
                transfer_id: pending.transfer_id,
                ack_byte_count: total_bytes,
                ack_chunk_count: total_chunks,
                audit_link: ctx.inbound_chain().current_link(),
            };
            ctx.push_outbound(OutboundFrame::Data {
                stream_id: header.stream_id,
                kind: OutboundStreamKind::Ack,
                chunk_index: total_chunks,
                payload: ack_codec::encode(&ack),
            });

            ctx.push_bulk_completion(header.stream_id, pending.transfer_id, total_bytes, total_chunks);
            let info = ctx.connection_info().clone();
            ctx.router().on_bulk_complete(
                &info, header.stream_id, pending.transfer_id,
                total_bytes, total_chunks,
            );

            ctx.remove_reassembler(header.stream_id);
            let _ = ctx.close_inbound_stream(header.stream_id);
        }
    }

    Ok(())
}
