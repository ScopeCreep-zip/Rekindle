//! STREAM_PAYLOAD handler — insert chunk into reassembler.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::ack as ack_codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::fin::PendingFinVerify;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::wire::outbound::{OutboundFrame, OutboundStreamKind};

pub fn handle(
    reassemblers: &mut HashMap<u8, Reassembler>,
    pending_fin_verify: &mut HashMap<u8, PendingFinVerify>,
    pending_bulk_deliveries: &mut Vec<(u8, u32, PlaintextBuf)>,
    pending_bulk_completions: &mut Vec<(u8, uuid::Uuid, u64, u32)>,
    stream_registry: &mut StreamRegistry,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let digest = *blake3::hash(payload).as_bytes();

    let reassembler = reassemblers.get_mut(&header.stream_id)
        .ok_or(HandlerError::StreamNotFound(header.stream_id))?;

    let delivered = reassembler.insert_with_digest(
        header.chunk_index, PlaintextBuf::Owned(payload.to_vec()), digest,
    );

    for (chunk_index, chunk_data) in delivered {
        pending_bulk_deliveries.push((header.stream_id, chunk_index, chunk_data));
    }

    if header.fin_follows() {
        if let Some(pending) = pending_fin_verify.remove(&header.stream_id) {
            let hash_ok = reassemblers.get(&header.stream_id)
                .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
                .unwrap_or(false);

            if !hash_ok {
                return Err(HandlerError::ContentHashMismatch);
            }

            let total_bytes = reassemblers.get(&header.stream_id)
                .map(|r| r.total_bytes()).unwrap_or(0);
            let total_chunks = reassemblers.get(&header.stream_id)
                .map(|r| r.next_expected()).unwrap_or(0);

            let ack = ack_codec::StreamAckPayload {
                transfer_id: pending.transfer_id,
                ack_byte_count: total_bytes,
                ack_chunk_count: total_chunks,
                audit_link: [0u8; 32], // filled from SharedAuditLinks by caller
            };
            outbound.push(OutboundFrame::Data {
                stream_id: header.stream_id,
                kind: OutboundStreamKind::Ack,
                chunk_index: total_chunks,
                payload: ack_codec::encode(&ack),
            });

            pending_bulk_completions.push((header.stream_id, pending.transfer_id, total_bytes, total_chunks));
            router.on_bulk_complete(info, header.stream_id, pending.transfer_id, total_bytes, total_chunks);
            reassemblers.remove(&header.stream_id);
            let _ = stream_registry.close(header.stream_id, crate::v4::stream::registry::Direction::Inbound);
        }
    }

    Ok(())
}
