use crate::v3::codec::datagram::reject as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let reject = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.resolve_pending_request(&reject.rejected_message_id);

    let info = ctx.connection_info().clone();
    ctx.router().on_reject(
        &info,
        reject.rejected_message_id,
        reject.reason_code as u32,
        &reject.detail,
    );

    Ok(())
}
