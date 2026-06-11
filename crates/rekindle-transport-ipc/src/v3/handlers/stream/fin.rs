//! STREAM_FIN handler — content hash verification and STREAM_ACK emission.
//!
//! Two paths:
//! 1. All chunks already present (inline path): verify immediately, emit
//!    STREAM_ACK, notify router.on_bulk_complete. This is the common case
//!    for small transfers where FIN arrives after all payloads.
//! 2. Chunks still missing (bulk/rayon path): store FIN metadata as
//!    PendingFinVerify. The control loop's check_deferred_fin_verify()
//!    fires verification when the last chunk arrives.

use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::fin as codec;
use crate::v3::codec::stream::ack as ack_codec;
use crate::v3::context::{OutboundFrame, OutboundStreamKind, PendingFinVerify, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let fin = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let stream_id = header.stream_id;

    tracing::debug!(
        stream_id,
        chunk_index = header.chunk_index,
        total_bytes = fin.total_bytes,
        content_hash = %hex::encode(&fin.final_content_hash[..8]),
        "fin::handle: FIN received via inline path"
    );

    let expected_total = header.chunk_index;
    let (next_exp, buffered) = ctx.reassembler_mut(stream_id)
        .map(|r| (r.next_expected(), r.buffered_count()))
        .unwrap_or((0, 0));
    let all_present = buffered == 0 && next_exp == expected_total;

    tracing::debug!(
        stream_id, expected_total, next_exp, buffered, all_present,
        "fin::handle: reassembler state at FIN arrival"
    );

    if all_present {
        // Immediate verification — all chunks are assembled
        let hash_ok = ctx.reassembler_mut(stream_id)
            .map(|r| r.verify_content_hash(&fin.final_content_hash).is_ok())
            .unwrap_or(false);

        if !hash_ok {
            return Err(HandlerError::ContentHashMismatch);
        }

        let total_bytes = ctx.reassembler_mut(stream_id)
            .map(|r| r.total_bytes())
            .unwrap_or(0);
        let total_chunks = ctx.reassembler_mut(stream_id)
            .map(|r| r.next_expected())
            .unwrap_or(0);

        // Emit STREAM_ACK
        let ack = ack_codec::StreamAckPayload {
            transfer_id: uuid::Uuid::nil(),
            ack_byte_count: total_bytes,
            ack_chunk_count: total_chunks,
            audit_link: ctx.inbound_chain().current_link(),
        };
        ctx.push_outbound(OutboundFrame::Data {
            stream_id,
            kind: OutboundStreamKind::Ack,
            chunk_index: total_chunks,
            payload: ack_codec::encode(&ack),
        });

        // Stage completion for the control loop's ordered drain.
        ctx.push_bulk_completion(stream_id, uuid::Uuid::nil(), total_bytes, total_chunks);
        // Notify application
        let info = ctx.connection_info().clone();
        ctx.router().on_bulk_complete(
            &info, stream_id, uuid::Uuid::nil(),
            total_bytes, total_chunks,
        );

        // Cleanup
        ctx.remove_reassembler(stream_id);
        let _ = ctx.close_inbound_stream(stream_id);
    } else {
        // Deferred verification — chunks still arriving (bulk/rayon path).
        // Store FIN metadata. The control loop's check_deferred_fin_verify()
        // fires verification when the reassembler has all chunks.
        ctx.store_pending_fin_verify(stream_id, PendingFinVerify {
            content_hash: fin.final_content_hash,
            audit_link: fin.final_audit_link,
            total_bytes: fin.total_bytes,
            transfer_id: uuid::Uuid::nil(),
            expected_chunks: expected_total,
        });

        // Transition stream state
        let _ = ctx.transition_inbound_stream(
            stream_id,
            crate::v3::stream::state::StreamEvent::FinSent,
        );
    }

    Ok(())
}
