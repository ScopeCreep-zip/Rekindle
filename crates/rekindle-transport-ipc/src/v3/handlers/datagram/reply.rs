use crate::v3::codec::datagram::reply as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let reply = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.resolve_pending_request(&reply.correlation_id);

    let info = ctx.connection_info().clone();
    ctx.router().on_reply(
        &info,
        reply.message_id,
        reply.correlation_id,
        reply.status_phase,
        &reply.application_payload,
    );

    Ok(())
}
