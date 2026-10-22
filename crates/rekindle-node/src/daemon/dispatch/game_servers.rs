//! A community's game-server requests (plan E3.1 H1), answered by the
//! Host's governance adapter on both hosts.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::IpcResponse;

pub(crate) use crate::host::adapters::governance::game_servers::NewGameServer;

use super::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// The community's game servers.
pub(crate) async fn handle_list(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).game_servers(&membership.governance_key).await {
        Ok(servers) => IpcResponse::ok(&servers),
        Err(e) => refused(&e, "game servers"),
    }
}

/// Add `server`; answered with its id.
pub(crate) async fn handle_add(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    server: NewGameServer,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .add_game_server(&membership.governance_key, server)
        .await
    {
        Ok(id) => IpcResponse::ok(&id),
        Err(e) => refused(&e, "game server add"),
    }
}

/// Remove the server `server_id`.
pub(crate) async fn handle_remove(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    server_id: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .remove_game_server(&membership.governance_key, server_id)
        .await
    {
        Ok(()) => IpcResponse::ok(&()),
        Err(e) => refused(&e, "game server remove"),
    }
}

/// A runtime error as the answer: 403 when we lack the permission.
fn refused(e: &GovernanceRuntimeError, what: &str) -> IpcResponse {
    match e {
        GovernanceRuntimeError::PermissionDenied => IpcResponse::error(403, format!("{what}: {e}")),
        _ => IpcResponse::error(500, format!("{what}: {e}")),
    }
}
