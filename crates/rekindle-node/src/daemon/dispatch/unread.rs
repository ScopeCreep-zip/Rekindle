//! Read markers and unread counts (plan E3.1 G1, D25), answered by the
//! Host's governance adapter on both hosts.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::UnreadRequest;

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// A read-marker or unread-count request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: UnreadRequest,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    match request {
        UnreadRequest::ChannelMarkRead {
            community,
            channel,
            up_to,
        } => {
            let membership = match ctx.resolve_community(&community) {
                Ok(m) => m,
                Err(e) => return e,
            };
            match adapter(ctx)
                .mark_channel_read(&membership.governance_key, &channel, &up_to)
                .await
            {
                Ok(()) => IpcResponse::ok(&()),
                Err(e) => runtime_refused(&e, "mark channel read"),
            }
        }
        UnreadRequest::UnreadCounts { community } => {
            let membership = match ctx.resolve_community(&community) {
                Ok(m) => m,
                Err(e) => return e,
            };
            match adapter(ctx).unread_counts(&membership.governance_key).await {
                Ok(counts) => IpcResponse::ok(&counts),
                Err(e) => runtime_refused(&e, "unread counts"),
            }
        }
    }
}
