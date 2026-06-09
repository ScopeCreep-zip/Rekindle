use crate::v3::codec::channel::backpressure as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;
use crate::v3::stream::flow_control::Severity;

pub fn handle_assert(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let bp = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let severity = match bp.severity {
        0x02 => Severity::Urgent,
        0x03 => Severity::Critical,
        _ => Severity::Advisory,
    };
    ctx.backpressure_mut().assert_backpressure(severity);
    Ok(())
}

pub fn handle_clear(ctx: &mut SessionContext, _payload: &[u8]) {
    ctx.backpressure_mut().clear();
}
