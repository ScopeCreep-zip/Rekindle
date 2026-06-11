use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::cancel as codec;
use crate::v3::codec::stream::cancel_ack as ack_codec;
use crate::v3::context::{OutboundFrame, OutboundStreamKind, SessionContext};
use crate::v3::handlers::HandlerError;
pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let cancel = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let audit_link = ctx.inbound_chain().current_link();

    // Extract the partial content hash from the reassembler BEFORE removing it.
    // This is the rolling BLAKE3 of all chunks received so far — the resume
    // verifier compares against it to ensure the resumed transfer picks up
    // from the exact byte boundary.
    let content_hash = ctx.reassembler_mut(header.stream_id)
        .map(|r| r.partial_content_hash())
        .unwrap_or([0u8; 32]);

    ctx.remove_reassembler(header.stream_id);
    let _ = ctx.close_inbound_stream(header.stream_id);

    if ctx.has_resume() {
        ctx.register_resume_state(
            cancel.transfer_id,
            cancel.bytes_through,
            cancel.chunks_through,
            audit_link,
            content_hash,
        );
    }

    let ack = ack_codec::StreamCancelAckPayload {
        transfer_id: cancel.transfer_id,
        bytes_through: cancel.bytes_through,
        chunks_through: cancel.chunks_through,
    };
    ctx.push_outbound(OutboundFrame::Data {
        stream_id: header.stream_id,
        kind: OutboundStreamKind::CancelAck,
        chunk_index: 0,
        payload: ack_codec::encode(&ack),
    });

    Ok(())
}
