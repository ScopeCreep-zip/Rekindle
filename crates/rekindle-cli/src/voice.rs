//! Voice commands: join, leave, status.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::VoiceCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(
    cmd: &VoiceCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        VoiceCmd::Join {
            community,
            channel,
            muted,
            deafened,
        } => {
            let value = client
                .request_ok(IpcRequest::VoiceJoin {
                    community: community.clone(),
                    channel: channel.clone(),
                    muted: *muted,
                    deafened: *deafened,
                })
                .await?;
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
        VoiceCmd::Mute | VoiceCmd::Deafen => {
            format::print_text("Mute/deafen toggling requires active voice session in TUI mode")
        }
    }
}
