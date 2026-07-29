//! CHANNEL_UNSUBSCRIBE handler.

use crate::v4::codec::channel::unsubscribe as codec;
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::handlers::HandlerError;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle(
    subscriptions: &mut SubscriptionRegistry,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let unsub = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    subscriptions.remove(&unsub.subscription_id);

    let ack = codec::UnsubscribeAckPayload {
        subscription_id: unsub.subscription_id,
    };
    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::UnsubscribeAck,
        payload: codec::encode_ack(&ack),
    });

    Ok(())
}
