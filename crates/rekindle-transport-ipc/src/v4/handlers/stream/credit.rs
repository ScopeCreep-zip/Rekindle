//! STREAM_CREDIT handler — per-stream credit replenishment.

use std::collections::HashMap;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::credit as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::stream::flow_control::CreditTracker;

pub fn handle(
    stream_credits: &mut HashMap<u8, CreditTracker>,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let credit = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if let Some(tracker) = stream_credits.get_mut(&header.stream_id) {
        tracker.replenish(credit.chunks, credit.generation);
    }

    Ok(())
}
