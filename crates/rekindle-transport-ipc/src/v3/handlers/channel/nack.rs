use crate::v3::codec::channel::nack as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let nack = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;
    ctx.resolve_pending_request(&nack.rejected_message_id);
    Ok(())
}
