//! Voice commands: join, leave, status, mute, deafen.

use crate::v2::prelude::{ChatRequest, DaemonRequest, LifecycleRequest};

use crate::v2::cli::VoiceCmd;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::prelude::DaemonClient;

pub async fn dispatch(cmd: &VoiceCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        VoiceCmd::Join { community, channel, muted, deafened } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::VoiceJoin {
                community: community.clone(),
                channel: channel.clone(),
                muted: *muted,
                deafened: *deafened,
            })).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Leave => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::VoiceLeave)).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Status => {
            let value = client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::NetworkStatus)).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Mute { on } => {
            let muted = on.unwrap_or(true);
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::VoiceMute { muted })).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Deafen { on } => {
            let deafened = on.unwrap_or(true);
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::VoiceDeafen { deafened })).await?;
            format::print_structured(&value, mode)
        }
    }
}
