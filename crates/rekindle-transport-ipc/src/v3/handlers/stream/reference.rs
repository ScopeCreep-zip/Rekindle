use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::ack as ack_codec;
use crate::v3::codec::stream::reference as codec;
use crate::v3::context::{OutboundFrame, OutboundStreamKind, SessionContext};
use crate::v3::dedup::reference::{receiver_decision, ReceiverDecision};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let reference = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let decision = receiver_decision(
        ctx.receiver_cache(),
        &reference.content_hash,
        reference.expected_total_bytes,
        reference.expected_chunk_count,
        reference.sender_clearance,
    );

    match decision {
        ReceiverDecision::CacheHit { payload: cached } => {
            let ack = ack_codec::StreamAckPayload {
                transfer_id: reference.transfer_id,
                ack_byte_count: cached.len() as u64,
                ack_chunk_count: reference.expected_chunk_count,
                audit_link: ctx.inbound_chain().current_link(),
            };
            ctx.push_outbound(OutboundFrame::Data {
                stream_id: header.stream_id,
                kind: OutboundStreamKind::Ack,
                chunk_index: 0,
                payload: ack_codec::encode(&ack),
            });
            Ok(())
        }
        ReceiverDecision::CacheMiss | ReceiverDecision::CacheMismatch => Err(HandlerError::DedupCacheMiss),
        ReceiverDecision::ClearanceDenied => Err(HandlerError::DedupClearanceInsufficient),
    }
}
