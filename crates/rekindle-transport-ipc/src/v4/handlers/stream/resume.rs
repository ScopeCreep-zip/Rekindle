//! STREAM_RESUME handler — evaluate resume request, reopen stream.

use std::collections::HashMap;
use std::sync::Arc;

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::codec::stream::resume as codec;
use crate::v4::codec::stream::resume_deny as deny_codec;
use crate::v4::config::SessionConfig;
use crate::v4::handlers::HandlerError;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::stream::flow_control::CreditTracker;
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::resume::{evaluate_resume_request, ResumeDecision, ResumeDenialReason, ResumeRegistry};
use crate::v4::wire::outbound::{OutboundFrame, OutboundStreamKind, OutboundFailureCode};

pub fn handle(
    stream_registry: &mut StreamRegistry,
    reassemblers: &mut HashMap<u8, Reassembler>,
    stream_credits: &mut HashMap<u8, CreditTracker>,
    early_bulk_chunks: &mut HashMap<u8, Vec<(u32, PlaintextBuf, [u8; 32])>>,
    pending_bulk_deliveries: &mut Vec<(u8, u32, PlaintextBuf)>,
    resume_registry: &ResumeRegistry,
    config: &Arc<SessionConfig>,
    outbound: &mut Vec<OutboundFrame>,
    header: &StreamHeaderInfo,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let resume = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let decision = evaluate_resume_request(
        resume_registry, resume.transfer_id,
        resume.anchor_audit_link, resume.anchor_content_hash,
    );

    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            stream_registry.open(header.stream_id, crate::v4::stream::registry::Direction::Inbound)
                .map_err(|_| HandlerError::StreamAlreadyOpen(header.stream_id))?;
            reassemblers.insert(
                header.stream_id,
                Reassembler::new_from_offset(config.reassembler_window, resume_from_chunk),
            );
            stream_credits.insert(
                header.stream_id,
                CreditTracker::new(config.initial_stream_credit_chunks, config.initial_lane_credit_bytes),
            );

            let early = early_bulk_chunks.remove(&header.stream_id).unwrap_or_default();
            for (chunk_index, data, digest) in early {
                if let Some(reassembler) = reassemblers.get_mut(&header.stream_id) {
                    let delivered = reassembler.insert_with_digest(chunk_index, data, digest);
                    for (ci, chunk_data) in delivered {
                        pending_bulk_deliveries.push((header.stream_id, ci, chunk_data));
                    }
                }
            }
            Ok(())
        }
        ResumeDecision::Denied(reason) => {
            let deny = deny_codec::StreamResumeDenyPayload {
                transfer_id: resume.transfer_id,
                reason_code: match &reason {
                    ResumeDenialReason::TransferIdUnknown => OutboundFailureCode::TransferIdUnknown,
                    ResumeDenialReason::ResumeWindowExpired => OutboundFailureCode::ResumeWindowExpired,
                    ResumeDenialReason::AuditAnchorMismatch => OutboundFailureCode::AuditAnchorMismatch,
                    ResumeDenialReason::ContentAnchorMismatch => OutboundFailureCode::ContentAnchorMismatch,
                },
            };
            outbound.push(OutboundFrame::Data {
                stream_id: header.stream_id,
                kind: OutboundStreamKind::ResumeDeny,
                chunk_index: 0,
                payload: deny_codec::encode(&deny),
            });
            Err(match reason {
                ResumeDenialReason::TransferIdUnknown => HandlerError::TransferIdUnknown(resume.transfer_id),
                ResumeDenialReason::ResumeWindowExpired => HandlerError::ResumeWindowExpired,
                ResumeDenialReason::AuditAnchorMismatch => HandlerError::AuditAnchorMismatch,
                ResumeDenialReason::ContentAnchorMismatch => HandlerError::ContentAnchorMismatch,
            })
        }
    }
}
