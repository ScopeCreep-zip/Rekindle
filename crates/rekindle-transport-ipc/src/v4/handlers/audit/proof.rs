//! AUDIT_PROOF handler — verify and store peer's proof.

use std::collections::HashMap;

use crate::v4::audit::VerifiedProof;
use crate::v4::codec::audit::proof as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::wire::outbound::OutboundFrame;

pub fn handle(
    verified_proofs: &mut HashMap<uuid::Uuid, VerifiedProof>,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let proof = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let verified = if proof.segment.is_empty() {
        proof.link == proof.anchor_link
    } else {
        proof.segment.last() == Some(&proof.link)
    };

    if !verified {
        outbound.push(OutboundFrame::channel_error(0x0007, "audit-chain-divergence"));
        return Err(HandlerError::AuditAnchorMismatch);
    }

    verified_proofs.insert(proof.query_id, VerifiedProof {
        query_id: proof.query_id,
        session_seq: proof.result_session_seq,
        link: proof.link,
        anchor_link: proof.anchor_link,
        verified: true,
    });

    Ok(())
}
