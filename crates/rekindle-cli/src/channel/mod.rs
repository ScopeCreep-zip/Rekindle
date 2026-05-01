//! Channel commands — list, create, delete, update, send, history, watch, pin/unpin.

mod create;
mod history;
mod list;
mod send;
mod watch;

use crate::cli::ChannelCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle channel <subcommand>`.
pub async fn dispatch(
    cmd: &ChannelCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ChannelCmd::List { community, format: fmt } => {
            let effective_mode = fmt
                .as_deref()
                .map_or(mode, |f| match f {
                    "json" => OutputMode::Json,
                    "jsonl" => OutputMode::Jsonl,
                    _ => mode,
                });
            list::cmd_list(handle, session, community, effective_mode).await
        }
        ChannelCmd::Create {
            community,
            name,
            kind,
            category,
            topic,
            slowmode,
        } => {
            create::cmd_create(
                handle,
                session,
                community,
                name,
                kind,
                category.as_deref(),
                topic.as_deref(),
                slowmode.unwrap_or(0),
                mode,
            )
            .await
        }
        ChannelCmd::Delete {
            community,
            channel,
            yes,
        } => {
            let membership = helpers::resolve_community(community, session)?;
            let channel_id = helpers::resolve_channel_id(channel);

            if !*yes {
                let confirmed = helpers::confirm(&format!("Delete channel '{channel}'?"))?;
                if !confirmed {
                    return format::print_text("Cancelled.");
                }
            }

            rekindle_transport::operations::channel_admin::delete_channel(
                handle.node(),
                &membership.governance_key,
                &channel_id,
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to delete channel: {e}"))?;

            if mode.is_structured() {
                format::print_structured(
                    &serde_json::json!({"status": "deleted", "channel": channel}),
                    mode,
                )
            } else {
                format::print_text(&format!("Channel '{channel}' deleted."))
            }
        }
        ChannelCmd::Update {
            community,
            channel,
            name,
            topic,
            slowmode,
        } => {
            let membership = helpers::resolve_community(community, session)?;
            let channel_id = helpers::resolve_channel_id(channel);

            let updated = rekindle_transport::operations::channel_admin::update_channel(
                handle.node(),
                &membership.governance_key,
                &channel_id,
                name.as_deref(),
                topic.as_deref(),
                *slowmode,
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to update channel: {e}"))?;

            if mode.is_structured() {
                format::print_structured(&updated, mode)
            } else {
                format::print_text(&format!("Channel '{}' updated.", updated.name))
            }
        }
        ChannelCmd::Send {
            community,
            channel,
            message,
            reply_to,
        } => {
            send::cmd_send(
                handle,
                session,
                community,
                channel,
                message,
                reply_to.as_deref(),
                mode,
            )
            .await
        }
        ChannelCmd::History {
            community,
            channel,
            limit,
            before: _,
            since: _,
            format: fmt,
        } => {
            let effective_mode = fmt
                .as_deref()
                .map_or(mode, |f| match f {
                    "json" => OutputMode::Json,
                    "jsonl" => OutputMode::Jsonl,
                    _ => mode,
                });
            history::cmd_history(handle, session, community, channel, *limit, effective_mode).await
        }
        ChannelCmd::Watch {
            community,
            channel,
            raw: _,
        } => watch::cmd_watch(handle, session, community, channel, mode).await,
        ChannelCmd::Pin {
            community: _,
            channel: _,
            msg_id: _,
        } => {
            // Pin/unpin operations will be implemented when the gossip
            // ceremony for MessagePinned control payloads is wired.
            format::print_text("Pin not yet implemented — coming in M3.")
        }
        ChannelCmd::Unpin {
            community: _,
            channel: _,
            msg_id: _,
        } => format::print_text("Unpin not yet implemented — coming in M3."),
    }
}
