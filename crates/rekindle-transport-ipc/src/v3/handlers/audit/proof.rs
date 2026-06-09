use crate::v3::codec::audit::proof as codec;
use crate::v3::context::{SessionContext, VerifiedProof};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let proof = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let verified = if proof.segment.is_empty() {
        proof.link == proof.anchor_link
    } else {
        proof.segment.last() == Some(&proof.link)
    };

    if !verified {
        ctx.push_channel_error(0x0007, "audit-chain-divergence");
        return Err(HandlerError::AuditAnchorMismatch);
    }

    ctx.store_verified_proof(VerifiedProof {
        query_id: proof.query_id,
        session_seq: proof.result_session_seq,
        link: proof.link,
        anchor_link: proof.anchor_link,
        verified: true,
    });

    Ok(())
}
