//! Voice commands: join, leave, status, mute, deafen.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::VoiceCmd;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch(cmd: &VoiceCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        VoiceCmd::Join { community, channel, muted, deafened } => {
            let value = client.request_ok(IpcRequest::VoiceJoin {
                community: community.clone(),
                channel: channel.clone(),
                muted: *muted,
                deafened: *deafened,
            }).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Leave => {
            let value = client.request_ok(IpcRequest::VoiceLeave).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Status => {
            let value = client.request_ok(IpcRequest::NetworkStatus).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Mute { on } => {
            // Explicit --on=true/false. If omitted, default to true (mute self).
            // True toggle requires querying current state which is a TUI concern.
            // CLI is imperative: `rekindle voice mute --on=false` to unmute.
            let muted = on.unwrap_or(true);
            let value = client.request_ok(IpcRequest::VoiceMute { muted }).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Deafen { on } => {
            let deafened = on.unwrap_or(true);
            let value = client.request_ok(IpcRequest::VoiceDeafen { deafened }).await?;
            format::print_structured(&value, mode)
        }
    }
}
