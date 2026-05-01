//! Direct messaging commands — send, inbox, watch, read.

mod inbox;
mod send;
mod watch;

use crate::cli::DmCmd;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle dm <subcommand>`.
pub async fn dispatch(
    cmd: &DmCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        DmCmd::Send {
            friend,
            message,
            file: _,
        } => send::cmd_send(handle, session, friend, message, mode).await,
        DmCmd::Inbox {
            friend,
            limit,
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
            inbox::cmd_inbox(handle, session, friend.as_deref(), *limit, effective_mode).await
        }
        DmCmd::Watch { friend } => {
            watch::cmd_watch(handle, session, friend.as_deref(), mode).await
        }
        DmCmd::Read {
            conversation_id,
            limit,
            before: _,
        } => inbox::cmd_read(handle, session, conversation_id, *limit, mode).await,
    }
}
