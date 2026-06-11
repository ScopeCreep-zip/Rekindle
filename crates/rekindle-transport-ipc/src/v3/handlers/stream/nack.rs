use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::nack as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::state::StreamEvent;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let nack = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let current_state = ctx.outbound_stream_state(header.stream_id);
    match current_state {
        Some(crate::v3::stream::state::StreamState::Opening) => {
            let _ = ctx.transition_outbound_stream(header.stream_id, StreamEvent::NackReceived);
        }
        _ => {
            tracing::warn!(
                stream_id = header.stream_id,
                reason = ?nack.reason_code,
                chunk = nack.rejected_chunk_idx,
                "stream NACK received"
            );
        }
    }

    Ok(())
}
