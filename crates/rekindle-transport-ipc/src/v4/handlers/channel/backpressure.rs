//! CHANNEL_BACKPRESSURE handlers — modify Data lane BackpressureState.
//!
//! Rerouted from Control lane to Data lane by wire::lane::processing_lane.

use crate::v4::codec::channel::backpressure as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::stream::flow_control::{BackpressureState, Severity};

pub fn handle_assert(
    backpressure: &mut BackpressureState,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let bp = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let severity = match bp.severity {
        0x02 => Severity::Urgent,
        0x03 => Severity::Critical,
        _ => Severity::Advisory,
    };
    backpressure.assert_backpressure(severity);
    Ok(())
}

pub fn handle_clear(backpressure: &mut BackpressureState) {
    backpressure.clear();
}
