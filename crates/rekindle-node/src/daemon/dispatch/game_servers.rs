//! A community's game-server requests (plan E3.1 H1), answered by the
//! Host's governance adapter on both hosts.

use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use crate::host::adapters::governance::game_servers::NewGameServer;

use super::{adapter, runtime_refused, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// A game-server request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    match request {
        CommunityRequest::GameServerList { community } => handle_list(ctx, state, &community).await,
        CommunityRequest::GameServerAdd {
            community,
            game_id,
            label,
            address,
        } => {
            let server = NewGameServer {
                game_id,
                label,
                address,
            };
            handle_add(ctx, state, &community, server).await
        }
        CommunityRequest::GameServerRemove {
            community,
            server_id,
        } => handle_remove(ctx, state, &community, &server_id).await,
        other => IpcResponse::error(
            500,
            format!("{} is not a game-server request", other.name()),
        ),
    }
}

/// The community's game servers.
async fn handle_list(ctx: &DaemonContext, state: DaemonState, community: &str) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).game_servers(&membership.governance_key).await {
        Ok(servers) => IpcResponse::ok(&servers),
        Err(e) => runtime_refused(&e, "game servers"),
    }
}

/// Add `server`; answered with its id.
async fn handle_add(
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
        Err(e) => runtime_refused(&e, "game server add"),
    }
}

/// Remove the server `server_id`.
async fn handle_remove(
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
        Err(e) => runtime_refused(&e, "game server remove"),
    }
}
