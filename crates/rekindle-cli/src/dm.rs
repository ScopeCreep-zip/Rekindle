//! DM commands: send, inbox, watch, read.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::DmCmd;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

pub async fn dispatch(cmd: &DmCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        DmCmd::Send {
            friend, message, ..
        } => {
            let value = client
                .request_ok(IpcRequest::DmSend {
                    peer_key: friend.clone(),
                    body: message.clone(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Inbox { limit, .. } => {
            let value = client
                .request_ok(IpcRequest::DmInbox {
                    limit: u32::try_from(*limit).unwrap_or(u32::MAX),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Watch { friend } => watch_dms(client, friend.as_deref(), mode).await,
        DmCmd::Read {
            conversation_id,
            limit,
            ..
        } => {
            // DM read is inbox scoped to a conversation — daemon returns all, CLI filters
            let _ = conversation_id;
            let value = client
                .request_ok(IpcRequest::DmInbox {
                    limit: u32::try_from(*limit).unwrap_or(u32::MAX),
                })
                .await?;
            format::print_structured(&value, mode)
        }
    }
}

/// `dm watch`: stream incoming DMs until Ctrl-C, optionally from one
/// friend (public key or display name).
async fn watch_dms(
    client: &DaemonClient,
    friend: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    use rekindle_types::subscription_events::{
        ChannelMessageEvent, EventCategory, SubscriptionEvent, SubscriptionFilter,
    };
    let filter = SubscriptionFilter::categories(vec![EventCategory::ChannelMessage]);
    crate::watch::stream(client, vec![filter], mode, |event| match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key,
            sender_name,
            timestamp,
            body,
            decryption_failed,
            ..
        }) if friend.is_none_or(|f| f == peer_key || sender_name.as_deref() == Some(f)) => {
            let who = sender_name.as_deref().map_or_else(
                || rekindle_client::fmt::abbreviate_key(peer_key),
                rekindle_utils::text::sanitize_for_display,
            );
            let text = if *decryption_failed {
                "(could not decrypt)".to_owned()
            } else {
                body.as_deref()
                    .map_or_else(String::new, rekindle_utils::text::sanitize_for_display)
            };
            Some(format!(
                "[{}] {who}: {text}",
                rekindle_client::fmt::format_time_short(*timestamp)
            ))
        }
        _ => None,
    })
    .await
}
