//! Voice commands: join, leave, status.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_utils::text::sanitize_for_display;

use crate::cli::VoiceCmd;
use crate::helpers;
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
            let value = helpers::send(
                client,
                IpcRequest::VoiceJoin {
                    community: community.clone(),
                    channel: channel.clone(),
                    muted: *muted,
                    deafened: *deafened,
                },
            )
            .await?;
            format::print_structured(&value, mode)?;
            if !watch {
                return Ok(());
            }
            let streamed = watch_voice(client, &value, mode).await;
            // Leave on Ctrl-C, and on a failed stream too: a session nobody
            // is watching should not stay joined.
            let left = helpers::send(client, IpcRequest::VoiceLeave).await;
            streamed?;
            left.map(drop)
        }
        VoiceCmd::Leave => {
            let value = helpers::send(client, IpcRequest::VoiceLeave).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Status => Err(crate::identity::unimplemented("voice status")),
        VoiceCmd::Mute { muted } => {
            let value = helpers::send(client, IpcRequest::VoiceSetMute { muted: *muted }).await?;
            format::print_structured(&value, mode)
        }
        VoiceCmd::Deafen { deafened } => {
            let value = helpers::send(
                client,
                IpcRequest::VoiceSetDeafen {
                    deafened: *deafened,
                },
            )
            .await?;
            format::print_structured(&value, mode)
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
    let mut names = std::collections::HashMap::new();
    crate::watch::stream(client, vec![filter], mode, |event| {
        let SubscriptionEvent::Voice(voice) = event else {
            return None;
        };
        match voice.scope() {
            Some(VoiceScope::Community { channel, .. }) if *channel == channel_id => {}
            _ => return None,
        }
        if mode.is_structured() {
            return Some(String::new());
        }
        Some(match voice {
            VoiceEvent::Joined {
                pseudonym,
                display_name: name,
                ..
            } => {
                let name = sanitize_for_display(name);
                let line = format!("{name} joined");
                names.insert(pseudonym.clone(), name);
                line
            }
            VoiceEvent::RosterUpdated { participants, .. } => {
                let mut roster = Vec::new();
                for participant in participants {
                    let name = sanitize_for_display(&participant.display_name);
                    names.insert(participant.pseudonym_key.clone(), name.clone());
                    roster.push(name);
                }
                format!("Voice roster: {}", roster.join(", "))
            }
            VoiceEvent::Left { pseudonym, .. } => {
                format!("{} left", names.remove(pseudonym)?)
            }
            VoiceEvent::MuteChanged {
                target_pseudonym,
                muted,
                ..
            } => format!(
                "{} {}",
                names.get(target_pseudonym)?,
                if *muted { "muted" } else { "unmuted" }
            ),
            other => serde_json::to_string(other).unwrap_or_default(),
        })
    })
    .await
}
