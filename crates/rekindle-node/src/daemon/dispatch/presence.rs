//! Presence and voice dispatch handlers.

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext};

pub(crate) async fn handle_set(
    ctx: &DaemonContext,
    state: DaemonState,
    status: &str,
    message: Option<&str>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    use rekindle_presence::UserStatusKind as S;
    let kind = match status {
        "online" => S::Online,
        "away" => S::Away,
        "busy" => S::Busy,
        "offline" => S::Offline,
        "invisible" => S::Invisible,
        other => return IpcResponse::error(400, format!("invalid status: {other}")),
    };
    if let Some(message) = message {
        let transport = match ctx.require_transport() {
            Ok(t) => t,
            Err(e) => return e,
        };
        let session = match ctx.require_session(Clone::clone) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if let Err(e) = rekindle_transport::operations::presence::set_status_message(
            &transport, &session, message,
        )
        .await
        {
            return IpcResponse::error(500, format!("status message failed: {e}"));
        }
    }
    // The unlock's STATUS publisher writes it (plan C7.8c).
    crate::daemon::status::set(ctx, kind);
    IpcResponse::ok(&serde_json::json!({ "status": status }))
}

pub(crate) async fn handle_game_set(
    ctx: &DaemonContext,
    state: DaemonState,
    game_name: &str,
    game_id: Option<u32>,
    elapsed_seconds: u32,
    server_address: Option<&str>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match rekindle_transport::operations::presence::set_game_presence(
        &transport,
        &session,
        game_name,
        game_id,
        elapsed_seconds,
        server_address,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "game": game_name })),
        Err(e) => IpcResponse::error(500, format!("game presence set failed: {e}")),
    }
}

pub(crate) async fn handle_game_clear(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match rekindle_transport::operations::presence::clear_game_presence(&transport, &session).await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "cleared": true })),
        Err(e) => IpcResponse::error(500, format!("game presence clear failed: {e}")),
    }
}

pub(crate) fn handle_voice_join(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    muted: bool,
    deafened: bool,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    match rekindle_transport::operations::voice::join_voice(
        &transport,
        &membership,
        channel,
        muted,
        deafened,
    ) {
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
    if !state.can_write() {
        return state_error(state, "write");
    }
    // The daemon keeps no voice session yet (`VoiceJoin` hands its session
    // back without holding it), so there is nothing to leave and no leave
    // to announce. Answering "left" would be a success that did not happen;
    // the daemon's voice engine arrives in plan E4.
    IpcResponse::error(501, "voice sessions are not held by the daemon yet")
}
