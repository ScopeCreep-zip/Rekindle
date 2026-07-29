//! STREAM_OPEN handler — open inbound stream, create reassembler.

use std::collections::HashMap;
use std::sync::Arc;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::open as codec;
use crate::v4::config::SessionConfig;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::fin::PendingFinVerify;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::stream::flow_control::CreditTracker;
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::wire::clearance::Clearance;

pub fn handle(
    stream_registry: &mut StreamRegistry,
    reassemblers: &mut HashMap<u8, Reassembler>,
    stream_credits: &mut HashMap<u8, CreditTracker>,
    pending_fin_verify: &mut HashMap<u8, PendingFinVerify>,
    early_bulk_chunks: &mut HashMap<u8, Vec<(u32, PlaintextBuf, [u8; 32])>>,
    pending_bulk_deliveries: &mut Vec<(u8, u32, PlaintextBuf)>,
    agreed_clearance: Clearance,
    config: &Arc<SessionConfig>,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let open = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if open.clearance_required > agreed_clearance {
        return Err(HandlerError::ClearanceInsufficient);
    }

    stream_registry.open(header.stream_id, crate::v4::stream::registry::Direction::Inbound)
        .map_err(|_| HandlerError::StreamAlreadyOpen(header.stream_id))?;

    reassemblers.insert(header.stream_id, Reassembler::new(config.reassembler_window));
    stream_credits.insert(
        header.stream_id,
        CreditTracker::new(config.initial_stream_credit_chunks, config.initial_lane_credit_bytes),
    );

    if open.expected_chunk_count == 1 {
        pending_fin_verify.insert(header.stream_id, PendingFinVerify {
            content_hash: open.content_hash,
            audit_link: [0u8; 32],
            total_bytes: open.expected_total_bytes,
            transfer_id: open.transfer_id,
            expected_chunks: 1,
        });
    }

    let early = early_bulk_chunks.remove(&header.stream_id).unwrap_or_default();
    for (chunk_index, data, digest) in early {
        if let Some(reassembler) = reassemblers.get_mut(&header.stream_id) {
            let delivered = reassembler.insert_with_digest(chunk_index, data, digest);
            for (ci, chunk_data) in delivered {
                pending_bulk_deliveries.push((header.stream_id, ci, chunk_data));
            }
        }
    }

    Ok(())
}
