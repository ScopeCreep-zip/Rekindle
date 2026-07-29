//! AUDIT_QUERY handler — construct proof from shared audit chain metadata.
//!
//! Returns a proof with current_link, anchor_link, and empty segment.
//! The peer verifies anchor → current_link without intermediate links.
//! Full-segment proofs (with per-link verification) require async access
//! to the audit merge task's chain history — out of scope for the
//! synchronous handler path.

use crate::v4::codec::audit::query as codec;
use crate::v4::codec::audit::proof as proof_codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SharedAuditLinks;
use crate::v4::wire::outbound::{OutboundFrame, OutboundAuditKind};

pub fn handle(
    audit_links: &SharedAuditLinks,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let query = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let current = audit_links.outbound_link.load();
    let anchor = audit_links.outbound_anchor.load();

    let proof = proof_codec::AuditProofPayload {
        query_id: query.query_id,
        result_session_seq: query.query_session_seq,
        link: current,
        anchor_link: anchor,
        segment: vec![],
    };

    outbound.push(OutboundFrame::Audit {
        kind: OutboundAuditKind::Proof,
        payload: proof_codec::encode(&proof),
    });

    Ok(())
}
