//! Group DM commands (plan E2.6): create, accept, decline, leave, rename, invite, send,
//! list and messages, each one Host request; members are named as
//! [`friend_ref`] reads them.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;

use crate::cli::GroupDmCmd;
use crate::friend_ref;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;

/// The request a subcommand makes, its friends named by key
/// ([`friend_ref::resolve_all`]).
async fn request(client: &DaemonClient, cmd: &GroupDmCmd) -> anyhow::Result<IpcRequest> {
    Ok(match cmd {
        GroupDmCmd::Create {
            name,
            members,
            origin_peer,
        } => IpcRequest::GroupDmCreate {
            name: name.clone(),
            members: friend_ref::resolve_all(client, members).await?,
            origin_peer: friend_ref::resolve_opt(client, origin_peer.as_deref()).await?,
        },
        GroupDmCmd::Accept { group_id } => IpcRequest::GroupDmAccept {
            group_id: group_id.clone(),
        },
        GroupDmCmd::Decline { group_id } => IpcRequest::GroupDmDecline {
            group_id: group_id.clone(),
        },
        GroupDmCmd::Leave { group_id } => IpcRequest::GroupDmLeave {
            group_id: group_id.clone(),
        },
        GroupDmCmd::Rename { group_id, name } => IpcRequest::GroupDmRename {
            group_id: group_id.clone(),
            name: name.clone(),
        },
        GroupDmCmd::Invite { group_id, members } => IpcRequest::GroupDmInvite {
            group_id: group_id.clone(),
            members: friend_ref::resolve_all(client, members).await?,
        },
        GroupDmCmd::Send { group_id, text } => IpcRequest::GroupDmSend {
            group_id: group_id.clone(),
            body: text.clone(),
        },
        GroupDmCmd::List => IpcRequest::GroupDmList,
        GroupDmCmd::Messages {
            group_id,
            limit,
            before,
        } => IpcRequest::GroupDmMessages {
            group_id: group_id.clone(),
            limit: *limit,
            before: *before,
        },
    })
}

pub async fn dispatch(
    cmd: &GroupDmCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    format::print_structured(
        &helpers::send(client, request(client, cmd).await?).await?,
        mode,
    )
}
