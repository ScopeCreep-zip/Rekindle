use crate::v3::codec::audit::query as codec;
use crate::v3::codec::audit::proof as proof_codec;
use crate::v3::context::{OutboundFrame, OutboundAuditKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let query = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let chain = ctx.outbound_chain();
    let links = chain.links();
    let anchor = chain.anchor_record().value;

    let seq_idx = usize::try_from(query.query_session_seq).expect("session_seq exceeds usize");
    let link = if seq_idx < links.len() {
        links[seq_idx]
    } else {
        chain.current_link()
    };

    let segment: Vec<[u8; 32]> = if seq_idx < links.len() {
        links[..=seq_idx].to_vec()
    } else {
        vec![]
    };

    let proof = proof_codec::AuditProofPayload {
        query_id: query.query_id,
        result_session_seq: query.query_session_seq,
        link,
        anchor_link: anchor,
        segment,
    };

    ctx.push_outbound(OutboundFrame::Audit {
        kind: OutboundAuditKind::Proof,
        payload: proof_codec::encode(&proof),
    });

    Ok(())
}
