//! Voice dispatch handlers (until E4.1 brings the daemon's voice engine).

use crate::daemon::DaemonState;
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
