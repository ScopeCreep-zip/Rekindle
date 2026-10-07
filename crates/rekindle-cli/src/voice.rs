//! Voice commands: join, leave, status.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::VoiceCmd;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

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
            watch,
        } => {
            let value = client
                .request_ok(IpcRequest::VoiceJoin {
                    community: community.clone(),
                    channel: channel.clone(),
                    muted: *muted,
                    deafened: *deafened,
                })
                .await?;
            format::print_structured(&value, mode)?;
            if !watch {
                return Ok(());
            }
            let streamed = watch_voice(client, &value, mode).await;
            // Leave on Ctrl-C, and on a failed stream too: a session nobody
            // is watching should not stay joined.
            let left = client.request_ok(IpcRequest::VoiceLeave).await;
            streamed?;
            left.map(drop).map_err(Into::into)
        }
        VoiceCmd::Leave => {
            let value = client.request_ok(IpcRequest::VoiceLeave).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Status => Err(crate::identity::unimplemented("voice status")),
        VoiceCmd::Mute | VoiceCmd::Deafen => {
            Err(crate::identity::unimplemented("voice mute/deafen"))
        }
    }
}

/// Stream the joined channel's voice events until Ctrl-C. `joined` is the
/// daemon's `VoiceJoin` answer, which names the channel by id.
async fn watch_voice(
    client: &DaemonClient,
    joined: &serde_json::Value,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{
        EventCategory, SubscriptionEvent, SubscriptionFilter, VoiceEvent, VoiceScope,
    };
    let channel_id = joined["channel"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the daemon's join answer names no channel"))?
        .to_owned();
    let filter = SubscriptionFilter::categories(vec![EventCategory::Voice]);
    crate::watch::stream(client, vec![filter], mode, |event| {
        let SubscriptionEvent::Voice(voice) = event else {
            return None;
        };
        match voice.scope() {
            Some(VoiceScope::Community { channel, .. }) if *channel == channel_id => {}
            _ => return None,
        }
        let who = |pseudonym: &str| rekindle_client::fmt::abbreviate_key(pseudonym);
        Some(match voice {
            VoiceEvent::Joined { pseudonym, .. } => format!("{} joined", who(pseudonym)),
            VoiceEvent::Left { pseudonym, .. } => format!("{} left", who(pseudonym)),
            VoiceEvent::MuteChanged {
                target_pseudonym,
                muted,
                ..
            } => format!(
                "{} {}",
                who(target_pseudonym),
                if *muted { "muted" } else { "unmuted" }
            ),
            other => serde_json::to_string(other).unwrap_or_default(),
        })
    })
    .await
}
