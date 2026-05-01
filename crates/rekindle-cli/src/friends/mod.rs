//! Friend lifecycle commands — add, accept, reject, list, remove, block, unblock.

mod accept;
mod add;
mod list;
mod remove;

use crate::cli::FriendCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle friend <subcommand>`.
pub async fn dispatch(
    cmd: &FriendCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        FriendCmd::Add { target, message } => {
            add::cmd_add(handle, session, target, message.as_deref(), mode).await
        }
        FriendCmd::Accept { request_id } => {
            accept::cmd_accept(handle, session, request_id, mode).await
        }
        FriendCmd::Reject { request_id } => {
            accept::cmd_reject(handle, session, request_id, mode).await
        }
        FriendCmd::List { status, format: fmt } => {
            let effective_mode = fmt
                .as_deref()
                .map_or(mode, |f| match f {
                    "json" => OutputMode::Json,
                    "jsonl" => OutputMode::Jsonl,
                    _ => mode,
                });
            list::cmd_list(handle, session, status.as_deref(), effective_mode).await
        }
        FriendCmd::Remove { friend, yes } => {
            remove::cmd_remove(handle, session, friend, *yes, mode).await
        }
        FriendCmd::Requests => cmd_requests(session, mode),
        FriendCmd::Block { friend } => {
            remove::cmd_block(handle, session, friend, mode).await
        }
        FriendCmd::Unblock { friend } => {
            remove::cmd_unblock(handle, session, friend, mode)
        }
    }
}

/// `rekindle friend requests` — list pending inbound friend requests.
fn cmd_requests(
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let pending = &session.pending_friend_requests;

    if mode.is_structured() {
        return format::print_structured(
            &pending
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "public_key": r.public_key,
                        "display_name": r.display_name,
                        "message": r.message,
                        "received_at": r.received_at,
                    })
                })
                .collect::<Vec<_>>(),
            mode,
        );
    }

    if pending.is_empty() {
        return format::print_text("No pending friend requests.");
    }

    format::print_text(&format!("Pending friend requests ({}):", pending.len()))?;
    format::print_text("")?;

    for req in pending {
        let name = crate::helpers::sanitize_for_display(&req.display_name);
        let key_short = crate::helpers::abbreviate_key(&req.public_key);
        let time = crate::helpers::format_timestamp(req.received_at);
        let msg = crate::helpers::sanitize_for_display(&req.message);

        format::print_text(&format!("  {name} ({key_short})"))?;
        format::print_text(&format!("    \"{msg}\""))?;
        format::print_text(&format!("    received: {time}"))?;
        format::print_text(&format!(
            "    accept: rekindle friend accept --request-id \"{}\"",
            req.public_key
        ))?;
        format::print_text(&format!(
            "    reject: rekindle friend reject --request-id \"{}\"",
            req.public_key
        ))?;
        format::print_text("")?;
    }

    Ok(())
}
