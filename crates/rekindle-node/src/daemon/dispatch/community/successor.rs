//! Successor communities (ADR 0011 "Successor community", plan E3.2
//! S7s.4): publish one, follow one, and the snapshot of those offered.

use rekindle_ipc::protocol::IpcResponse;

use super::lifecycle::{activate_community, join_failure_response};
use crate::daemon::dispatch::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Re-create `anchor` under a successor anchor and name it here; answers
/// the new community as a create does.
pub(crate) async fn handle_publish_successor(
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
    let created = Box::pin(adapter(ctx).publish_successor(&membership.anchor)).await;
    match created {
        Ok(created) => {
            if ctx.shell.is_none() {
                activate_joined(ctx, &created.anchor).await;
            }
            IpcResponse::ok(&created)
        }
        Err(e) => IpcResponse::error(409, e),
    }
}

/// Follow a successor of `anchor` with `invite` (validated to open it):
/// both links are re-checked, then the join runs; `anchor` is kept.
pub(crate) async fn handle_follow_successor(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    invite: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let link = match rekindle_types::invite::InviteLink::parse(invite) {
        Ok(link) => link,
        Err(e) => return IpcResponse::error(400, format!("successor invite: {e}")),
    };
    let joined = match Box::pin(adapter(ctx).follow_successor(&membership.anchor, &link)).await {
        Ok(joined) => joined,
        Err(failure) => return join_failure_response(failure),
    };
    if ctx.shell.is_none() {
        activate_joined(ctx, &joined.anchor).await;
    }
    IpcResponse::ok(&joined)
}

/// The successors of `anchor` offered this session.
pub(crate) fn handle_successors(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    match ctx.resolve_community(anchor) {
        Ok(m) => IpcResponse::ok(&adapter(ctx).successor_offers(&m.anchor)),
        Err(e) => e,
    }
}

/// Bring a community this request created or joined up, as a create or
/// join does without a shell (`lifecycle::handle_join`).
async fn activate_joined(ctx: &DaemonContext, anchor: &str) {
    let membership = ctx
        .session
        .read()
        .as_ref()
        .and_then(|s| s.community(anchor).cloned());
    if let Some(membership) = membership {
        activate_community(ctx, &membership).await;
    }
}
