//! Presence commands: set status, game presence, clear game, watch (streaming).

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::cli::PresenceCmd;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::transport::DaemonClient;

pub async fn dispatch(cmd: &PresenceCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        PresenceCmd::Set { status, message, .. } => {
            let value = client.request_ok(IpcRequest::PresenceSet {
                status: status.clone(),
                message: message.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        PresenceCmd::Game { game_name, game_id, elapsed_seconds, server_address } => {
            let value = client.request_ok(IpcRequest::GamePresenceSet {
                game_name: game_name.clone(),
                game_id: *game_id,
                elapsed_seconds: *elapsed_seconds,
                server_address: server_address.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        PresenceCmd::GameClear => {
            let value = client.request_ok(IpcRequest::GamePresenceClear).await?;
            format::print_structured(&value, mode)
        }
        PresenceCmd::Watch { .. } => {
            anyhow::bail!("presence watch: use the streaming path in main dispatch")
        }
    }
}

/// Streaming presence watch — subscribe to events and print JSONL.
pub async fn watch_streaming(
    client: &DaemonClient,
    event_rx: &mut tokio::sync::mpsc::Receiver<rekindle_types::subscription_events::SubscriptionEvent>,
    community_filter: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{SubscriptionEvent, PresenceEvent};

    if let Some(community) = community_filter {
        client.subscribe_scoped(community).await?;
    } else {
        client.subscribe_all().await?;
    }

    if !mode.is_structured() {
        format::print_text("Watching presence changes  (Ctrl+C to stop)")?;
    }

    loop {
        match event_rx.recv().await {
            Some(SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
                ref community, ref pseudonym, ref status, ref game_name, game_id,
            })) => {
                if community_filter.is_some_and(|f| f != community) {
                    continue;
                }
                let obj = serde_json::json!({
                    "type": "community_presence",
                    "community": community,
                    "pseudonym": pseudonym,
                    "status": status,
                    "game_name": game_name,
                    "game_id": game_id,
                });
                format::print_structured(&obj, mode)?;
            }
            Some(SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                ref peer_key, ref status, ref game_name,
            })) => {
                let obj = serde_json::json!({
                    "type": "friend_presence",
                    "peer_key": peer_key,
                    "status": status,
                    "game_name": game_name,
                });
                format::print_structured(&obj, mode)?;
            }
            None => break,
            _ => {}
        }
    }
    Ok(())
}
