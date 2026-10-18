//! Our own presence, profile, game and audit log: one request each to the
//! host, which owns them (plan E1.1).

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::presence::SessionStatus;

use crate::cli::{AuditCmd, GameCmd, PresenceCmd, PresenceStatus, ProfileCmd};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

/// The largest avatar file read: the bus's frame.
const MAX_AVATAR_FILE: u64 = rekindle_ipc::framing::MAX_APP_FRAME as u64;

pub async fn dispatch(
    cmd: &PresenceCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        PresenceCmd::Set { status } => IpcRequest::PresenceSet {
            status: match status {
                PresenceStatus::Online => SessionStatus::Online,
                PresenceStatus::Away => SessionStatus::Away,
                PresenceStatus::Busy => SessionStatus::Busy,
                PresenceStatus::Invisible => SessionStatus::Invisible,
            },
        },
        PresenceCmd::Message { text } => IpcRequest::PresenceSetMessage {
            message: text.clone(),
        },
        PresenceCmd::Show => IpcRequest::PresenceGet,
        PresenceCmd::Watch { .. } => IpcRequest::Status,
    };
    let value = helpers::send(client, request).await?;
    format::print_structured(&value, mode)
}

pub async fn dispatch_profile(
    cmd: &ProfileCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        ProfileCmd::Nickname { name } => IpcRequest::ProfileSetNickname {
            display_name: name.clone(),
        },
        ProfileCmd::Avatar { file } => {
            let size = std::fs::metadata(file)?.len();
            if size > MAX_AVATAR_FILE {
                anyhow::bail!(
                    "{} is {size} bytes; at most {MAX_AVATAR_FILE}",
                    file.display()
                );
            }
            IpcRequest::ProfileSetAvatar {
                image: std::fs::read(file)?,
            }
        }
    };
    let value = helpers::send(client, request).await?;
    format::print_structured(&value, mode)
}

pub async fn dispatch_game(
    cmd: &GameCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        GameCmd::Set { name, id, server } => IpcRequest::GamePresenceSet {
            game_name: name.clone(),
            game_id: *id,
            elapsed_seconds: 0,
            server_address: server.clone(),
        },
        GameCmd::Clear => IpcRequest::GamePresenceClear,
    };
    let value = helpers::send(client, request).await?;
    format::print_structured(&value, mode)
}

pub async fn dispatch_audit(
    cmd: &AuditCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        AuditCmd::Verify => IpcRequest::AuditVerify,
        AuditCmd::Export { since } => IpcRequest::AuditExport { since: *since },
    };
    let value = helpers::send(client, request).await?;
    format::print_structured(&value, mode)
}

/// `undo <id>`: the inverse the host offered.
pub async fn undo(id: &str, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let id: rekindle_types::intent::IntentId =
        serde_json::from_value(serde_json::Value::String(id.to_owned()))
            .map_err(|e| anyhow::anyhow!("not an undo id: {e}"))?;
    let value = helpers::send(client, IpcRequest::Undo { id }).await?;
    format::print_structured(&value, mode)
}
