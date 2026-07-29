//! DATAGRAM_NOTIFY handler.

use crate::v4::codec::channel::ack as ack_codec;
use crate::v4::codec::datagram::notify as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle(
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let notify = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    router.on_notify(info, notify.message_id, notify.sender_clearance, &notify.application_payload);

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(notify.message_id),
    });

    Ok(())
}
