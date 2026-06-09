use crate::v3::codec::channel::unsubscribe as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let unsub = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.remove_subscription(&unsub.subscription_id);

    let ack = codec::UnsubscribeAckPayload {
        subscription_id: unsub.subscription_id,
    };
    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::UnsubscribeAck,
        payload: codec::encode_ack(&ack),
    });

    Ok(())
}
