//! Community transfer (ADR 0011 "Successor community", plan E3.2 S7s.4):
//! a creator offers the community to a member or withdraws the offer; the
//! member accepts it through the creator-link round. Replaces the old
//! `TransferOwnership`, which granted an administrator role and moved no
//! creator authority.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::governance::OpHash;
use rekindle_types::id::PseudonymKey;

use crate::daemon::dispatch::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Offer `anchor` to `transfer_to`.
pub(crate) async fn handle_offer_transfer(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    transfer_to: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let to = PseudonymKey::from_hex_lossy(transfer_to);
    match adapter(ctx).offer_transfer(&membership.anchor, &to).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "offered": transfer_to })),
        Err(e) => IpcResponse::error(409, e),
    }
}

/// Withdraw our standing offer of `anchor`.
pub(crate) async fn handle_withdraw_transfer(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).withdraw_transfer(&membership.anchor).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "withdrawn": true })),
        Err(e) => IpcResponse::error(409, e),
    }
}

/// Accept the standing offer of `anchor` to us; answers the reserved
/// successor with the creators still to link (lead decision (46)).
pub(crate) async fn handle_accept_transfer(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match Box::pin(adapter(ctx).accept_transfer(&membership.anchor)).await {
        Ok(created) => IpcResponse::ok(&created),
        Err(e) => IpcResponse::error(409, e),
    }
}

/// Answer the link request `request` (op hash, hex) in `anchor` that asks
/// us (lead decision (46)).
pub(crate) async fn handle_answer_successor_link(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    request: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let request = OpHash(PseudonymKey::from_hex_lossy(request).0);
    match Box::pin(adapter(ctx).answer_successor_link(&membership.anchor, request)).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "linked": true })),
        Err(e) => IpcResponse::error(409, e),
    }
}
