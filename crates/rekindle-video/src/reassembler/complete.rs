//! Buffer hygiene and frame completion (including FEC recovery).

use super::{
    reconstruct_frame, ReassembledFrame, ReassemblerError, StreamBuffer,
    MAX_PENDING_FRAMES_PER_STREAM, STALE_FRAME_HORIZON_MS, STREAM_ID_LEN,
};

pub(super) fn evict_stale(buffer: &mut StreamBuffer, now_ms: u32) {
    buffer.frames.retain(|_, partial| {
        now_ms.saturating_sub(partial.received_at_ms) <= STALE_FRAME_HORIZON_MS
    });
}

pub(super) fn cap_pending(buffer: &mut StreamBuffer, frame_seq: u32) {
    if !buffer.frames.contains_key(&frame_seq)
        && buffer.frames.len() >= MAX_PENDING_FRAMES_PER_STREAM
    {
        if let Some((&oldest_seq, _)) = buffer.frames.iter().min_by_key(|(_, p)| p.received_at_ms) {
            buffer.frames.remove(&oldest_seq);
        }
    }
}

/// Try to complete `frame_seq`. Returns the assembled payload via
/// (a) direct concatenation when every data shard arrived, or
/// (b) Reed-Solomon reconstruction when data + parity ≥ data_count
/// and at least one data shard is missing.
pub(super) fn try_complete(
    buffer: &mut StreamBuffer,
    frame_seq: u32,
    stream_id: [u8; STREAM_ID_LEN],
) -> Result<Option<ReassembledFrame>, ReassemblerError> {
    let Some(partial) = buffer.frames.get_mut(&frame_seq) else {
        return Ok(None);
    };

    // Fast path: every data shard present — concatenate.
    if partial.received_data_count == partial.frag_total {
        let mut payload = Vec::new();
        for chunk in &mut partial.chunks {
            if let Some(bytes) = chunk.take() {
                payload.extend(bytes);
            }
        }
        // Systematic-code invariant: data fragments carry the REAL
        // (unpadded) object bytes (see `fragment_frame_with_fec`), so
        // the in-order concat of all data shards IS the exact ciphertext
        // — no frame_len truncation needed or wanted. The old
        // truncate-only-if-frame_len>0 leaked FEC padding into the
        // ciphertext when data arrived before parity (frame_len==0),
        // failing AEAD decrypt. Padding survives only inside
        // `reconstruct_frame`, where RS genuinely needs padded shards.
        let frame = ReassembledFrame {
            stream_id,
            frame_seq,
            keyframe: partial.keyframe.unwrap_or(false),
            codec: partial.codec,
            timestamp: partial.timestamp,
            mek_generation: partial.mek_generation,
            payload,
            recovered_via_fec: false,
        };
        buffer.frames.remove(&frame_seq);
        return Ok(Some(frame));
    }

    // FEC path: have parity, and total received ≥ data_count, with at
    // least one data shard missing. Need parity_chunks initialized
    // (otherwise the sender shipped no FEC).
    if partial.parity_chunks.is_empty() {
        return Ok(None);
    }
    let total_received = partial.received_data_count + partial.received_parity_count;
    if total_received < partial.frag_total {
        return Ok(None);
    }

    let parity_total = u8::try_from(partial.parity_chunks.len()).unwrap_or(u8::MAX);
    let received_data: Vec<(u8, Vec<u8>)> = partial
        .chunks
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|p| (u8::try_from(idx).expect("frag_total fits u8"), p.clone()))
        })
        .collect();
    let received_parity: Vec<(u8, Vec<u8>)> = partial
        .parity_chunks
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|p| (u8::try_from(idx).expect("parity_total fits u8"), p.clone()))
        })
        .collect();
    let payload = reconstruct_frame(
        &received_data,
        &received_parity,
        partial.frag_total,
        parity_total,
        partial.frame_len,
    )
    .map_err(|e| ReassemblerError::FecReconstruct(e.to_string()))?;

    let frame = ReassembledFrame {
        stream_id,
        frame_seq,
        keyframe: partial.keyframe.unwrap_or(false),
        codec: partial.codec,
        timestamp: partial.timestamp,
        mek_generation: partial.mek_generation,
        payload,
        recovered_via_fec: true,
    };
    buffer.frames.remove(&frame_seq);
    Ok(Some(frame))
}
