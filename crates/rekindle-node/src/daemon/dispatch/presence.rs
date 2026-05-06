//! Presence and voice dispatch handlers.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{DaemonContext, state_error};

pub(crate) async fn handle_set(
    ctx: &DaemonContext, state: DaemonState, status: &str, message: Option<&str>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    if let Err(e) = validation::validate_status(status) { return e; }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::presence::set_status(
        &transport, &session, status, message,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "status": status })),
        Err(e) => IpcResponse::error(500, format!("presence set failed: {e}")),
    }
}

pub(crate) async fn handle_game_set(
    ctx: &DaemonContext, state: DaemonState,
    game_name: &str, game_id: Option<u32>, elapsed_seconds: u32, server_address: Option<&str>,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::presence::set_game_presence(
        &transport, &session, game_name, game_id, elapsed_seconds, server_address,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "game": game_name })),
        Err(e) => IpcResponse::error(500, format!("game presence set failed: {e}")),
    }
}

pub(crate) async fn handle_game_clear(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::presence::clear_game_presence(&transport, &session).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "cleared": true })),
        Err(e) => IpcResponse::error(500, format!("game presence clear failed: {e}")),
    }
}

pub(crate) async fn handle_voice_join(
    ctx: &DaemonContext, state: DaemonState,
    community: &str, channel: &str, muted: bool, deafened: bool,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let membership = match ctx.resolve_community(community) { Ok(m) => m, Err(e) => return e };

    match rekindle_transport::operations::voice::join_voice(
        &transport, &membership, channel, &ctx.mek_cache, muted, deafened,
    ).await {
        Ok(session) => IpcResponse::ok(&serde_json::json!({
            "joined": true,
            "community": session.community_id,
            "channel": session.channel_id,
            "muted": session.muted,
            "deafened": session.deafened,
        })),
        Err(e) => IpcResponse::error(500, format!("voice join failed: {e}")),
    }
}

pub(crate) fn handle_voice_leave(_ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    // Voice leave is local state cleanup — the transport operation is sync
    // and the gossip broadcast is the caller's responsibility.
    IpcResponse::ok(&serde_json::json!({ "left": true }))
}
