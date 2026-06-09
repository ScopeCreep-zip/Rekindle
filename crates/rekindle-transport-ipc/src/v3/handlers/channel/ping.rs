use crate::v3::codec::channel::ping as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let ping = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.set_remote_last_seen_our_seq(ping.last_seen_remote_seq);

    // Echo the PING's sender_epoch_ns verbatim so the PING originator
    // can compute RTT = now - echoed_epoch_ns. Per SCTP heartbeat
    // pattern: the ACK echoes the entire heartbeat info including
    // the sent_at timestamp.
    let pong = codec::PingPayload {
        ping_nonce: ping.ping_nonce,
        sender_epoch_ns: ping.sender_epoch_ns,
        last_seen_remote_seq: ctx.recv_last_seq(),
    };

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::Pong,
        payload: codec::encode(&pong),
    });

    Ok(())
}
