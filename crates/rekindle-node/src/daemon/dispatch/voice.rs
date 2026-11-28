//! Voice dispatch handlers (until E4.1 brings the daemon's voice engine):
//! the session a join establishes is the Host's own record
//! (`host::own_voice`), and every join and leave writes our beat at once
//! (`adapters::presence::voice_changed`, plan E3.4 "Constant rate"), so
//! the members' voice rosters list us from it.

use crate::daemon::DaemonState;
use crate::host::adapters::presence::voice_changed;
use rekindle_ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext};

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

    let session = match rekindle_transport::operations::voice::join_voice(
        &transport,
        &membership,
        channel,
        muted,
        deafened,
    ) {
        Ok(session) => session,
        Err(e) => return IpcResponse::error(500, format!("voice join failed: {e}")),
    };
    let reply = serde_json::json!({
        "joined": true,
        "community": session.community_id,
        "channel": session.channel_id,
        "muted": session.muted,
        "deafened": session.deafened,
    });
    let joined = session.community_id.clone();
    // One channel at a time: a session in another community ends, and our
    // beat there stops naming its channel.
    if let Some(replaced) = ctx.own_voice.join(session) {
        if replaced.community_id != joined {
            voice_changed(ctx, &replaced.community_id);
        }
    }
    voice_changed(ctx, &joined);
    IpcResponse::ok(&reply)
}

pub(crate) fn handle_voice_leave(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    // Answering "left" while in no channel would be a success that did not
    // happen.
    let Some(session) = ctx.own_voice.leave() else {
        return IpcResponse::error(409, "not in a voice channel");
    };
    voice_changed(ctx, &session.community_id);
    IpcResponse::ok(&serde_json::json!({
        "left": true,
        "community": session.community_id,
        "channel": session.channel_id,
    }))
}
