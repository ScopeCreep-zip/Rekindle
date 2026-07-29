//! STREAM_CANCEL handler — receiver cancels, stores resume state.

use std::collections::HashMap;
use std::sync::Arc;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::cancel as codec;
use crate::v4::codec::stream::cancel_ack as ack_codec;
use crate::v4::config::SessionConfig;
use crate::v4::handlers::HandlerError;
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::resume::ResumeRegistry;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::outbound::{OutboundFrame, OutboundStreamKind};

pub fn handle(
    reassemblers: &mut HashMap<u8, Reassembler>,
    stream_registry: &mut StreamRegistry,
    resume_registry: &mut ResumeRegistry,
    active_capabilities: CapabilityBits,
    outbound: &mut Vec<OutboundFrame>,
    config: &Arc<SessionConfig>,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let cancel = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let content_hash = reassemblers.get(&header.stream_id)
        .map(|r| r.partial_content_hash())
        .unwrap_or([0u8; 32]);

    reassemblers.remove(&header.stream_id);
    let _ = stream_registry.close(header.stream_id, crate::v4::stream::registry::Direction::Inbound);

    if active_capabilities.contains(CapabilityBits::RESUME) {
        use std::time::Instant;
        use crate::v4::stream::resume::ResumeState;
        resume_registry.register(ResumeState::new(
            cancel.transfer_id,
            cancel.bytes_through,
            cancel.chunks_through,
            [0u8; 32], // audit_link — read from SharedAuditLinks by caller if needed
            content_hash,
            Instant::now(),
            config.resume_config.eligibility_window,
        ));
    }

    let ack = ack_codec::StreamCancelAckPayload {
        transfer_id: cancel.transfer_id,
        bytes_through: cancel.bytes_through,
        chunks_through: cancel.chunks_through,
    };
    outbound.push(OutboundFrame::Data {
        stream_id: header.stream_id,
        kind: OutboundStreamKind::CancelAck,
        chunk_index: 0,
        payload: ack_codec::encode(&ack),
    });

    Ok(())
}
