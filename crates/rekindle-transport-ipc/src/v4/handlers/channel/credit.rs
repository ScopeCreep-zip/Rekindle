//! CHANNEL_CREDIT handler — applies credit updates to Data lane state.
//!
//! Rerouted from Control lane to Data lane by wire::lane::processing_lane.
//! Credit frames modify stream_credits and lane_credit_bytes which are
//! owned exclusively by the Data lane task.

use std::collections::HashMap;

use crate::v4::codec::channel::credit as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::stream::flow_control::CreditTracker;

pub fn handle(
    stream_credits: &mut HashMap<u8, CreditTracker>,
    lane_credit_bytes: &mut HashMap<u8, u32>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let credit = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    match credit.scope {
        0x01 => {
            lane_credit_bytes.insert(credit.lane_or_stream_id, credit.credit_bytes);
        }
        0x02 => {
            if let Some(tracker) = stream_credits.get_mut(&credit.lane_or_stream_id) {
                tracker.replenish(credit.credit_frames, credit.credit_generation);
            }
        }
        _ => {}
    }

    Ok(())
}
