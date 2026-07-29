//! CHANNEL_PING handler — echo back as PONG.

use crate::v4::codec::channel::ping as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

pub fn handle(
    outbound: &mut Vec<OutboundFrame>,
    remote_last_seen_our_seq: &mut u64,
    recv_last_seq: u64,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let ping = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    *remote_last_seen_our_seq = ping.last_seen_remote_seq;

    let pong = codec::PingPayload {
        ping_nonce: ping.ping_nonce,
        sender_epoch_ns: ping.sender_epoch_ns,
        last_seen_remote_seq: recv_last_seq,
    };

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::Pong,
        payload: codec::encode(&pong),
    });

    Ok(())
}
