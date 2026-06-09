use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::open as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::registry::Direction;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    // No session state check here — dispatch_frame (inbound.rs) is the SSOT
    // for frame admission via is_frame_allowed(). Handlers must not duplicate
    // that check. The epoch-tagged rotation protocol allows in-flight stream
    // operations during the Rotating state because both epoch key sets are
    // simultaneously active.

    let open = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if open.clearance_required > ctx.agreed_clearance() {
        return Err(HandlerError::ClearanceInsufficient);
    }

    tracing::debug!(
        stream_id = header.stream_id,
        role = ?ctx.role(),
        "stream::open handler: opening stream in registry"
    );
    ctx.stream_registry_mut().open(header.stream_id, Direction::Inbound)
        .map_err(|e| {
            tracing::error!(
                stream_id = header.stream_id,
                role = ?ctx.role(),
                error = %e,
                "stream::open handler: registry open FAILED"
            );
            HandlerError::StreamAlreadyOpen(header.stream_id)
        })?;

    ctx.create_reassembler(header.stream_id);

    // For single-chunk transfers using FIN_FOLLOWS, store verification
    // metadata now. The PAYLOAD handler (or handle_bulk_decrypted) will
    // use this to verify and ACK immediately without waiting for a
    // separate FIN frame. For multi-chunk transfers, FIN carries this data.
    if open.expected_chunk_count == 1 {
        ctx.store_pending_fin_verify(header.stream_id, crate::v3::context::PendingFinVerify {
            content_hash: open.content_hash,
            audit_link: [0u8; 32], // populated at verify time from inbound chain
            total_bytes: open.expected_total_bytes,
            transfer_id: open.transfer_id,
            expected_chunks: 1,
        });
    }

    // Replay any early bulk chunks that arrived before this STREAM_OPEN.
    // The rayon decrypt path can deliver chunks faster than the inline
    // signal bridge delivers STREAM_OPEN. Those chunks were buffered in
    // early_bulk_chunks by handle_bulk_decrypted.
    let early = ctx.drain_early_chunks(header.stream_id);
    if !early.is_empty() {
        tracing::debug!(
            stream_id = header.stream_id,
            count = early.len(),
            "stream::open: replaying early bulk chunks into new reassembler"
        );
        for (chunk_index, data, digest) in early {
            if let Some(reassembler) = ctx.reassembler_mut(header.stream_id) {
                let delivered = reassembler.insert_with_digest(chunk_index, data, digest);
                for (ci, chunk_data) in delivered {
                    ctx.push_bulk_delivery(header.stream_id, ci, chunk_data);
                }
            }
        }
    }

    Ok(())
}
