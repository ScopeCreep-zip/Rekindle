//! STREAM_SACK handler — selective acknowledgement, releases retention.

use std::collections::HashMap;

use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::sack as codec;
use crate::v4::handlers::HandlerError;

pub fn handle(
    chunk_to_seq: &HashMap<(u8, u32), u64>,
    retention: &mut RetentionBuffer,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let sack = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    for (byte_idx, &byte) in sack.sack_bitmap.iter().enumerate() {
        for bit in 0..8u32 {
            if byte & (1 << bit) != 0 {
                let byte_offset = u32::try_from(byte_idx).expect("sack bitmap index exceeds u32");
                let chunk_index = sack.cumulative_through + 1 + byte_offset * 8 + bit;
                if let Some(session_seq) = chunk_to_seq.get(&(header.stream_id, chunk_index)) {
                    retention.remove(*session_seq);
                }
            }
        }
    }

    Ok(())
}
