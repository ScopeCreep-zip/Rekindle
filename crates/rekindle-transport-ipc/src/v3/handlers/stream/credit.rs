use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::credit as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let credit = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if let Some(tracker) = ctx.stream_credit_tracker_mut(header.stream_id) {
        tracker.replenish(credit.chunks, credit.generation);
    }

    Ok(())
}
