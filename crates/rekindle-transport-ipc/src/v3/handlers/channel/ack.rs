use crate::v3::codec::channel::ack as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let ack = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    for message_id in &ack.message_ids {
        ctx.resolve_pending_request(message_id);
    }

    let info = ctx.connection_info().clone();
    ctx.router().on_ack(&info, &ack.message_ids);

    Ok(())
}
