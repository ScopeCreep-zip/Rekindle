//! Voice requests use the held session in the Host voice adapter.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::audio::VoiceFailure;
use rekindle_types::dto::voice::{VoiceJoined, VoiceLeft};

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::voice;

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
    let community = match ctx.resolve_community(community) {
        Ok(membership) => membership.anchor,
        Err(response) => return response,
    };
    match voice::join(ctx, &community, channel, muted, deafened).await {
        Ok((muted, deafened)) => IpcResponse::ok(&VoiceJoined {
            joined: true,
            community,
            channel: channel.to_owned(),
            muted,
            deafened,
        }),
        Err(notice) => IpcResponse::error(409, notice),
    }
}

pub(crate) async fn handle_voice_leave(ctx: &DaemonContext) -> IpcResponse {
    match voice::leave(ctx).await {
        Some((community, channel)) => IpcResponse::ok(&VoiceLeft {
            left: true,
            community,
            channel,
        }),
        None => IpcResponse::error(409, VoiceFailure::NotConnected.notice()),
    }
}

pub(crate) async fn handle_voice_mute(ctx: &DaemonContext, muted: bool) -> IpcResponse {
    match voice::set_mute(ctx, muted).await {
        Some(controls) => IpcResponse::ok(&controls),
        None => IpcResponse::error(409, VoiceFailure::NotConnected.notice()),
    }
}

pub(crate) async fn handle_voice_deafen(ctx: &DaemonContext, deafened: bool) -> IpcResponse {
    match voice::set_deafen(ctx, deafened).await {
        Some(controls) => IpcResponse::ok(&controls),
        None => IpcResponse::error(409, VoiceFailure::NotConnected.notice()),
    }
}
