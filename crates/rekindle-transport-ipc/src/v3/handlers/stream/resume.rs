use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::codec::stream::resume as codec;
use crate::v3::codec::stream::resume_deny as deny_codec;
use crate::v3::context::{OutboundFrame, OutboundStreamKind, OutboundFailureCode, SessionContext};
use crate::v3::handlers::HandlerError;
use crate::v3::stream::registry::Direction;
use crate::v3::stream::resume::{evaluate_resume_request, ResumeDecision, ResumeDenialReason};

pub fn handle(ctx: &mut SessionContext, header: &StreamHeaderInfo, payload: &[u8]) -> Result<(), HandlerError> {
    let resume = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let decision = evaluate_resume_request(
        ctx.resume_registry(),
        resume.transfer_id,
        resume.anchor_audit_link,
        resume.anchor_content_hash,
    );

    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            ctx.stream_registry_mut().open(header.stream_id, Direction::Inbound)
                .map_err(|_| HandlerError::StreamAlreadyOpen(header.stream_id))?;
            ctx.create_reassembler_from_offset(header.stream_id, resume_from_chunk);
            // Replay early bulk chunks (same race as STREAM_OPEN)
            let early = ctx.drain_early_chunks(header.stream_id);
            for (chunk_index, data, digest) in early {
                if let Some(reassembler) = ctx.reassembler_mut(header.stream_id) {
                    let delivered = reassembler.insert_with_digest(chunk_index, data, digest);
                    for (ci, chunk_data) in delivered {
                        ctx.push_bulk_delivery(header.stream_id, ci, chunk_data);
                    }
                }
            }
            Ok(())
        }
        ResumeDecision::Denied(reason) => {
            let deny = deny_codec::StreamResumeDenyPayload {
                transfer_id: resume.transfer_id,
                reason_code: match reason {
                    ResumeDenialReason::TransferIdUnknown => OutboundFailureCode::TransferIdUnknown,
                    ResumeDenialReason::ResumeWindowExpired => OutboundFailureCode::ResumeWindowExpired,
                    ResumeDenialReason::AuditAnchorMismatch => OutboundFailureCode::AuditAnchorMismatch,
                    ResumeDenialReason::ContentAnchorMismatch => OutboundFailureCode::ContentAnchorMismatch,
                },
            };
            ctx.push_outbound(OutboundFrame::Data {
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
