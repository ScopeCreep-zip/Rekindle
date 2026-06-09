use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::io::lane_channels::PlaintextBuf;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let digest = *blake3::hash(payload).as_bytes();

    let reassembler = ctx.reassembler_mut(header.stream_id)
        .ok_or(HandlerError::StreamNotFound(header.stream_id))?;

    let delivered = reassembler.insert_with_digest(
        header.chunk_index, PlaintextBuf::Owned(payload.to_vec()), digest,
    );

    for (chunk_index, chunk_data) in delivered {
        ctx.push_bulk_delivery(header.stream_id, chunk_index, chunk_data);
    }

    Ok(())
}
