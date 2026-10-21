//! Group DM commands (plan E2.6): create, accept, decline, leave, list and
//! messages, each one Host request.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;

use crate::cli::GroupDmCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;

fn request(cmd: &GroupDmCmd) -> IpcRequest {
    match cmd {
        GroupDmCmd::Create { name, members } => IpcRequest::GroupDmCreate {
            name: name.clone(),
            members: members.clone(),
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
    }
}

pub async fn dispatch(
    cmd: &GroupDmCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    format::print_structured(&helpers::send(client, request(cmd)).await?, mode)
}
