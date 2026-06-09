use crate::v3::codec::audit::checkpoint as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let cp = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let local_link = ctx.inbound_chain().current_link();
    let local_length = ctx.inbound_chain().length();
    let local_anchor = ctx.inbound_chain().anchor_record().value;

    if cp.anchor_link != local_anchor {
        ctx.push_channel_error(0x0007, "audit-chain-divergence");
        return Err(HandlerError::AuditAnchorMismatch);
    }

    if cp.chain_length == local_length && cp.chain_link != local_link {
        ctx.push_channel_error(0x0007, "audit-chain-divergence");
        return Err(HandlerError::AuditAnchorMismatch);
    }

    Ok(())
}
