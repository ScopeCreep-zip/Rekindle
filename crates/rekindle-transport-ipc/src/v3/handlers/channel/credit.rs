use crate::v3::codec::channel::credit as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let credit = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    match credit.scope {
        0x01 => {
            // Lane-scoped
            ctx.set_lane_credit_bytes(credit.lane_or_stream_id, credit.credit_bytes);
        }
        0x02 => {
            // Stream-scoped — silently ignore if stream not open
            if let Some(tracker) = ctx.stream_credit_tracker_mut(credit.lane_or_stream_id) {
                tracker.replenish(credit.credit_frames, credit.credit_generation);
            }
        }
        _ => {} // unknown scope, ignore
    }

    Ok(())
}
