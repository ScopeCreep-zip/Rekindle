use crate::v3::codec::channel::subscribe as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    if ctx.subscription_count() >= ctx.max_subscriptions() {
        return Err(HandlerError::SubscriptionQuotaExhausted);
    }

    let sub = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let topic_hashes: Vec<[u8; 32]> = sub.topics.iter().map(|t| t.topic_hash).collect();
    ctx.register_subscription(sub.subscription_id, &topic_hashes, &sub.conditions);

    let ack = codec::SubscribeAckPayload {
        subscription_id: sub.subscription_id,
        accepted_hashes: topic_hashes,
    };
    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::SubscribeAck,
        payload: codec::encode_ack(&ack),
    });

    Ok(())
}
