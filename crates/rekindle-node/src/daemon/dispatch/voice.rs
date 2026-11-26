//! Voice dispatch handlers: the Host's one voice engine (plan E4.1 S5a2,
//! `host::adapters::voice`).

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use super::{state_error, DaemonContext};

pub(crate) async fn handle_voice_join(
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
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let ctx = ctx.this.upgrade().expect("a context in use is alive");
    match crate::host::adapters::voice::join(ctx, &membership.anchor, channel, muted, deafened)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({
            "joined": true,
            "community": membership.anchor,
            "channel": channel,
            "muted": muted,
            "deafened": deafened,
        })),
        Err(reason) => IpcResponse::error(409, reason),
    }
}

/// Leave is always available; with no session live there is nothing to
/// leave (`left: false`).
pub(crate) async fn handle_voice_leave(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let ctx = ctx.this.upgrade().expect("a context in use is alive");
    let left = crate::host::adapters::voice::leave(ctx).await;
    IpcResponse::ok(&serde_json::json!({ "left": left }))
}
