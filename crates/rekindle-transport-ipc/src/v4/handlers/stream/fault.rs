//! STREAM_FAULT handler — insert fault chunk into reassembler.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::handlers::HandlerError;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::stream::reassembler::Reassembler;

pub fn handle(
    reassemblers: &mut HashMap<u8, Reassembler>,
    pending_bulk_deliveries: &mut Vec<(u8, u32, PlaintextBuf)>,
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

    Ok(())
}
