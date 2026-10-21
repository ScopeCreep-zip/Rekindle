//! DM commands: send, inbox, watch, history, read marks.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::DmCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

pub async fn dispatch(cmd: &DmCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        DmCmd::Send {
            friend, message, ..
        } => {
            let value = helpers::send(
                client,
                IpcRequest::DmSend {
                    peer_key: friend.clone(),
                    body: message.clone(),
                    reply_to: None,
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Inbox { limit, .. } => {
            let value = helpers::send(
                client,
                IpcRequest::DmInbox {
                    limit: u32::try_from(*limit).unwrap_or(u32::MAX),
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Watch { friend } => watch_dms(client, friend.as_deref(), mode).await,
        DmCmd::History {
            friend,
            limit,
            before,
        } => {
            let value = helpers::send(
                client,
                IpcRequest::DmHistory {
                    peer_key: friend.clone(),
                    limit: *limit,
                    before: *before,
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Read { friend } => {
            let value = helpers::send(
                client,
                IpcRequest::DmMarkRead {
                    peer_key: friend.clone(),
                },
            )
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
        ChannelMessageEvent, EventCategory, FriendEvent, SubscriptionEvent, SubscriptionFilter,
    };
    // Friend events too: a pin mismatch (plan E2.4, B3) refuses every DM.
    let filter =
        SubscriptionFilter::categories(vec![EventCategory::ChannelMessage, EventCategory::Friend]);
    crate::watch::stream(client, vec![filter], mode, |event| match event {
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key,
            sender_name,
            timestamp,
            body,
            decryption_failed,
            is_own,
            ..
        }) if friend.is_none_or(|f| f == peer_key || sender_name.as_deref() == Some(f)) => {
            let who = if *is_own {
                "you".to_owned()
            } else {
                sender_name.as_deref().map_or_else(
                    || rekindle_client::fmt::abbreviate_key(peer_key),
                    rekindle_utils::text::sanitize_for_display,
                )
            };
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
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
            conversation_id,
            client_id,
            state,
        }) if friend.is_none_or(|f| f == conversation_id) => Some(format!(
            "message {} to {}: {}",
            rekindle_client::fmt::abbreviate_key(client_id),
            rekindle_client::fmt::abbreviate_key(conversation_id),
            format!("{state:?}").to_lowercase()
        )),
        SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch {
            peer_key,
            display_name,
        }) if friend.is_none_or(|f| f == peer_key || f == display_name) => Some(format!(
            "! {}'s keys are not signed by the identity you know. Nothing was sent; verify \
             their safety number.",
            rekindle_utils::text::sanitize_for_display(display_name)
        )),
        // Their identity rotated (plan audit R-5): name the new key, which
        // is the one to watch from now on.
        SubscriptionEvent::Friend(FriendEvent::KeyChanged { peer_key, new_key })
            if friend.is_none_or(|f| f == peer_key) =>
        {
            Some(format!(
                "! {} moved to a new identity key {}; verify their safety number. Watch the new \
                 key to follow them.",
                rekindle_client::fmt::abbreviate_key(peer_key),
                rekindle_client::fmt::abbreviate_key(new_key)
            ))
        }
        _ => None,
    })
    .await
}
