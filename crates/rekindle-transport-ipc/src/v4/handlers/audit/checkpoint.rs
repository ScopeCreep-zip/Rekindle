//! AUDIT_CHECKPOINT handler — verify peer's chain state against local.

use std::sync::atomic::Ordering;

use crate::v4::codec::audit::checkpoint as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::shared_state::SharedAuditLinks;
use crate::v4::wire::outbound::OutboundFrame;

pub fn handle(
    audit_links: &SharedAuditLinks,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let cp = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let local_link = audit_links.inbound_link.load();
    let local_length = audit_links.inbound_length.load(Ordering::Acquire);
    let local_anchor = audit_links.inbound_anchor.load();

    if cp.anchor_link != local_anchor {
        outbound.push(OutboundFrame::channel_error(0x0007, "audit-chain-divergence"));
        return Err(HandlerError::AuditAnchorMismatch);
    }

    if cp.chain_length == local_length && cp.chain_link != local_link {
        outbound.push(OutboundFrame::channel_error(0x0007, "audit-chain-divergence"));
        return Err(HandlerError::AuditAnchorMismatch);
    }

    Ok(())
}
