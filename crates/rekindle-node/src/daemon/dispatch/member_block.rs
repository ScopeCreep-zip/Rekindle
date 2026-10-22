//! A community block's requests (plan E3.1 F3, D26), answered by the Host's
//! channel adapter on both hosts.

use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::channel::{refusal, HostChannel};

/// A `MemberBlock`, `MemberUnblock` or `MemberBlockList` request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    let read = matches!(request, CommunityRequest::MemberBlockList { .. });
    if read && !state.can_query() {
        return state_error(state, "query");
    }
    if !read && !state.can_write() {
        return state_error(state, "write");
    }
    let community = match &request {
        CommunityRequest::MemberBlock { community, .. }
        | CommunityRequest::MemberUnblock { community, .. }
        | CommunityRequest::MemberBlockList { community } => community.clone(),
        other => {
            return IpcResponse::error(500, format!("{} is not a block request", other.name()))
        }
    };
    let key = match ctx.resolve_community(&community) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    let channel = HostChannel::new(ctx);
    match request {
        CommunityRequest::MemberBlock {
            target_pseudonym, ..
        } => match channel.block_member(&key, &target_pseudonym).await {
            Ok(()) => IpcResponse::ok(&()),
            Err(e) => refusal(&e),
        },
        CommunityRequest::MemberUnblock {
            target_pseudonym, ..
        } => match channel.unblock_member(&key, &target_pseudonym).await {
            Ok(()) => IpcResponse::ok(&()),
            Err(e) => refusal(&e),
        },
        _ => match channel.blocked_members(&key).await {
            Ok(blocked) => IpcResponse::ok(&blocked),
            Err(e) => refusal(&e),
        },
    }
}
