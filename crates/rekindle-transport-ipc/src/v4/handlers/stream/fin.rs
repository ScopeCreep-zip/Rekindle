//! STREAM_FIN handler — content hash verification and STREAM_ACK emission.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::ack as ack_codec;
use crate::v4::codec::stream::fin as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::fin::PendingFinVerify;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::wire::outbound::{OutboundFrame, OutboundStreamKind};

pub fn handle(
    reassemblers: &mut HashMap<u8, Reassembler>,
    stream_registry: &mut StreamRegistry,
    pending_fin_verify: &mut HashMap<u8, PendingFinVerify>,
    pending_bulk_completions: &mut Vec<(u8, uuid::Uuid, u64, u32)>,
    outbound: &mut Vec<OutboundFrame>,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let fin = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let stream_id = header.stream_id;
    let expected_total = header.chunk_index;

    let (next_exp, buffered) = reassemblers.get(&stream_id)
        .map(|r| (r.next_expected(), r.buffered_count()))
        .unwrap_or((0, 0));
    let all_present = buffered == 0 && next_exp == expected_total;

    tracing::info!(
        stream_id,
        expected_total,
        next_exp,
        buffered,
        all_present,
        total_bytes = fin.total_bytes,
        content_hash = %hex::encode(&fin.final_content_hash[..8]),
        "FIN handler: received STREAM_FIN"
    );

    if all_present {
        let hash_ok = reassemblers.get(&stream_id)
            .map(|r| r.verify_content_hash(&fin.final_content_hash).is_ok())
            .unwrap_or(false);

        if !hash_ok {
            tracing::error!(stream_id, "content hash mismatch at FIN");
            router.on_bulk_failed(
                info, stream_id, uuid::Uuid::nil(),
                "content hash verification failed",
            );
            reassemblers.remove(&stream_id);
            let _ = stream_registry.close(
                stream_id,
                crate::v4::stream::registry::Direction::Inbound,
            );
            return Err(HandlerError::ContentHashMismatch);
        }

        let total_bytes = reassemblers.get(&stream_id)
            .map(|r| r.total_bytes()).unwrap_or(0);
        let total_chunks = reassemblers.get(&stream_id)
            .map(|r| r.next_expected()).unwrap_or(0);

        let ack = ack_codec::StreamAckPayload {
            transfer_id: uuid::Uuid::nil(),
            ack_byte_count: total_bytes,
            ack_chunk_count: total_chunks,
            audit_link: [0u8; 32], // filled by audit merge via SharedAuditLinks
        };
        outbound.push(OutboundFrame::Data {
            stream_id,
            kind: OutboundStreamKind::Ack,
            chunk_index: total_chunks,
            payload: ack_codec::encode(&ack),
        });

        pending_bulk_completions.push((stream_id, uuid::Uuid::nil(), total_bytes, total_chunks));
        router.on_bulk_complete(info, stream_id, uuid::Uuid::nil(), total_bytes, total_chunks);
        reassemblers.remove(&stream_id);
        let _ = stream_registry.close(stream_id, crate::v4::stream::registry::Direction::Inbound);
    } else {
        tracing::info!(
            stream_id,
            expected_total,
            next_exp,
            buffered,
            "FIN handler: deferring verification — not all chunks present"
        );
        pending_fin_verify.insert(stream_id, PendingFinVerify {
            content_hash: fin.final_content_hash,
            audit_link: fin.final_audit_link,
            total_bytes: fin.total_bytes,
            transfer_id: uuid::Uuid::nil(),
            expected_chunks: expected_total,
        });

        let _ = stream_registry.transition(
            stream_id,
            crate::v4::stream::registry::Direction::Inbound,
            crate::v4::stream::state::StreamEvent::FinSent,
        );
    }

    Ok(())
}
