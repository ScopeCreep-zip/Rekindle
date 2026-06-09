use crate::v3::codec::channel::ack as ack_codec;
use crate::v3::codec::datagram::notify as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let notify = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let info = ctx.connection_info().clone();
    ctx.router().on_notify(
        &info,
        notify.message_id,
        notify.sender_clearance,
        &notify.application_payload,
    );

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(notify.message_id),
    });

    Ok(())
}
