use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::sack as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let sack = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    for (byte_idx, &byte) in sack.sack_bitmap.iter().enumerate() {
        for bit in 0..8u32 {
            if byte & (1 << bit) != 0 {
                let byte_offset = u32::try_from(byte_idx).expect("sack bitmap index exceeds u32");
                let chunk_index = sack.cumulative_through + 1 + byte_offset * 8 + bit;
                if let Some(session_seq) = ctx.chunk_session_seq(header.stream_id, chunk_index) {
                    ctx.retention_mut().remove(session_seq);
                }
            }
        }
    }

    Ok(())
}
