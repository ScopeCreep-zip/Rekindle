//! Bulk transfer FIN lifecycle — send-side emission and receive-side verification.
//!
//! Send side: `emit_stream_fin` encodes STREAM_FIN when all payload
//! LinkInputs have been received, sending it through the crossbeam
//! bulk wire channel (same FIFO path as payload chunks).
//!
//! Receive side: `check_deferred_fin_verify` fires Merkle hash
//! verification when the reassembler has all chunks and a PendingFinVerify
//! exists. Emits STREAM_ACK or STREAM_NACK.

use std::sync::Arc;

use crate::v3::codec::stream::{ack as ack_codec, fin as fin_codec};
use crate::v3::context::{OutboundFrame, PendingFinState, PendingFinVerify, SessionContext};
use crate::v3::io::encode::FrameEncoder;
use crate::v3::io::lane_channels::BulkFrame;
use crate::v3::stream::registry::Direction;
use crate::v3::stream::state::StreamEvent;
use crate::v3::wire::frame_kind::StreamKind;

use super::audit_reorder::AuditReorderBuffer;
use super::drain;

/// Encode and emit STREAM_FIN through the crossbeam bulk wire channel.
///
/// Called when the outbound audit reorder buffer has received all N
/// payload LinkInputs for a stream. The outbound chain is fully advanced,
/// so `current_link()` is the correct final_audit_link.
///
/// FIN is dispatched to a rayon worker which encodes it into wire bytes
/// and sends them through the crossbeam bulk wire channel. The FIN's own
/// LinkInput is returned via oneshot and inserted directly into the
/// outbound reorder buffer — no crossbeam send on the tokio thread.
pub(super) async fn emit_stream_fin(
    ctx: &mut SessionContext,
    encoder: &Arc<FrameEncoder>,
    encrypt_pool: &Arc<rayon::ThreadPool>,
    outbound_reorder: &mut AuditReorderBuffer,
    stream_id: u8,
    completed: &PendingFinState,
) {
    let _ = ctx.stream_registry_mut().transition(stream_id, Direction::Outbound, StreamEvent::FinSent);

    let audit_link = ctx.outbound_chain().current_link();

    let fin_payload = fin_codec::encode(&fin_codec::StreamFinPayload {
        total_bytes: completed.total_bytes,
        fault_count: 0,
        final_content_hash: completed.content_hash,
        final_audit_link: audit_link,
    });

    let frame = OutboundFrame::Data {
        stream_id,
        kind: StreamKind::Fin,
        chunk_index: completed.chunk_count,
        payload: fin_payload,
    };

    // Dispatch FIN encode to rayon. The worker encodes into wire bytes
    // (Vec<u8>), sends through crossbeam bulk channel, and returns the
    // LinkInput via oneshot for audit chain advancement.
    let (link_tx, link_rx) = tokio::sync::oneshot::channel();
    let enc = Arc::clone(encoder);
    let bulk_wire_tx = completed.bulk_wire_tx.clone();

    encrypt_pool.spawn(move || {
        let ewa = enc.encode_with_audit(&frame);
        let wire_bytes = ewa.encoded.into_wire_bytes();
        tracing::debug!(stream_id, wire_len = wire_bytes.len(), "emit_stream_fin: rayon encoded FIN");

        if bulk_wire_tx.send(BulkFrame::Plain(wire_bytes)).is_err() {
            tracing::error!(stream_id, "emit_stream_fin: bulk_wire_tx closed during FIN send");
            return;
        }
        tracing::debug!(stream_id, "emit_stream_fin: FIN wire bytes sent to write task");

        let _ = link_tx.send(ewa.link_input);
        tracing::debug!(stream_id, "emit_stream_fin: LinkInput sent via oneshot");
    });

    // Await the FIN's LinkInput and insert directly into the outbound
    // reorder buffer. No crossbeam send — the control loop is on a tokio
    // worker thread and must never call blocking send.
    match link_rx.await {
        Ok(link_input) => {
            drain::insert_and_advance_outbound(ctx, outbound_reorder, link_input);
        }
        Err(_) => {
            tracing::error!(stream_id, "FIN rayon worker panicked — LinkInput lost");
        }
    }
}

/// Check if any stream has both a pending FIN verify and a complete reassembler.
/// If so, verify Merkle content hash, emit STREAM_ACK or STREAM_NACK, and clean up.
pub(super) fn check_deferred_fin_verify(ctx: &mut SessionContext) {
    let stream_ids: Vec<u8> = (0..=255u8)
        .filter(|&sid| ctx.has_pending_fin_verify(sid) && ctx.has_reassembler(sid))
        .collect();

    for stream_id in stream_ids {
        let next_expected = ctx.reassembler_mut(stream_id).map(|r| r.next_expected()).unwrap_or(0);
        let buffered = ctx.reassembler_mut(stream_id).map(|r| r.buffered_count()).unwrap_or(0);
        let total_bytes = ctx.reassembler_mut(stream_id).map(|r| r.total_bytes()).unwrap_or(0);

        // Peek at expected_chunks without consuming the PendingFinVerify.
        // Cannot take() here — if the reassembler isn't complete yet, we
        // need PendingFinVerify to stay for the next check_deferred call.
        let expected_chunks = match ctx.pending_fin_verify_ref(stream_id) {
            Some(p) => p.expected_chunks,
            None => continue,
        };

        // The reassembler is complete when ALL payload chunks (0..expected_chunks-1)
        // have been delivered in order (next_expected >= expected_chunks) and no
        // out-of-order chunks remain buffered.
        let reassembler_complete = buffered == 0 && next_expected >= expected_chunks;

        tracing::debug!(
            stream_id, next_expected, expected_chunks, buffered, total_bytes, reassembler_complete,
            "check_deferred_fin_verify: checking stream"
        );

        if !reassembler_complete {
            continue;
        }

        let pending = match ctx.take_pending_fin_verify(stream_id) {
            Some(p) => p,
            None => continue,
        };

        tracing::debug!(
            stream_id,
            pending_content_hash = %hex::encode(&pending.content_hash[..8]),
            pending_total_bytes = pending.total_bytes,
            pending_expected_chunks = pending.expected_chunks,
            "check_deferred_fin_verify: verifying content hash"
        );

        let hash_ok = ctx.reassembler_mut(stream_id)
            .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
            .unwrap_or(false);

        if hash_ok {
            emit_ack(ctx, stream_id, &pending);
        } else {
            emit_nack(ctx, stream_id, &pending);
        }
    }
}

fn emit_ack(ctx: &mut SessionContext, stream_id: u8, pending: &PendingFinVerify) {
    let reassembled_bytes = ctx.reassembler_mut(stream_id).map(|r| r.total_bytes()).unwrap_or(0);
    let total_chunks = ctx.reassembler_mut(stream_id).map(|r| r.next_expected()).unwrap_or(0);

    // Verify total_bytes consistency
    if pending.total_bytes > 0 && pending.total_bytes != reassembled_bytes {
        emit_nack_with_detail(ctx, stream_id, pending, &format!(
            "total_bytes mismatch: FIN={} reassembled={}", pending.total_bytes, reassembled_bytes
        ));
        return;
    }

    let ack = ack_codec::StreamAckPayload {
        transfer_id: pending.transfer_id,
        ack_byte_count: reassembled_bytes,
        ack_chunk_count: total_chunks,
        audit_link: ctx.inbound_chain().current_link(),
    };
    ctx.push_outbound(OutboundFrame::Data {
        stream_id,
        kind: StreamKind::Ack,
        chunk_index: total_chunks,
        payload: ack_codec::encode(&ack),
    });

    ctx.push_bulk_completion(stream_id, pending.transfer_id, reassembled_bytes, total_chunks);
    let info = ctx.connection_info().clone();
    ctx.router().on_bulk_complete(&info, stream_id, pending.transfer_id, reassembled_bytes, total_chunks);
    ctx.remove_reassembler(stream_id);
    let _ = ctx.stream_registry_mut().close(stream_id, Direction::Inbound);
}

fn emit_nack(ctx: &mut SessionContext, stream_id: u8, pending: &PendingFinVerify) {
    emit_nack_with_detail(ctx, stream_id, pending, "content hash verification failed");
}

fn emit_nack_with_detail(ctx: &mut SessionContext, stream_id: u8, pending: &PendingFinVerify, detail: &str) {
    let nack_payload = crate::v3::codec::stream::nack::encode(
        &crate::v3::codec::stream::nack::StreamNackPayload {
            reason_code: crate::v3::wire::failure::FailureCode::ContentHashMismatch,
            rejected_chunk_idx: 0,
            detail: detail.to_owned(),
        },
    );
    ctx.push_outbound(OutboundFrame::Data {
        stream_id, kind: StreamKind::Nack, chunk_index: 0, payload: nack_payload,
    });
    let info = ctx.connection_info().clone();
    ctx.router().on_bulk_failed(&info, stream_id, pending.transfer_id, detail);
    ctx.remove_reassembler(stream_id);
    let _ = ctx.stream_registry_mut().close(stream_id, Direction::Inbound);
}
