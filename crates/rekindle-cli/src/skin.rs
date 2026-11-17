//! `rekindle skin …` (plan E7.7.2), one request each.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;

use crate::cli::SkinCmd;
use crate::helpers;
use crate::output::{format, OutputMode};

pub async fn dispatch(
    cmd: &SkinCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let value = helpers::send(client, request(cmd)?).await?;
    format::print_structured(&value, mode)
}

/// The request `cmd` sends.
pub fn request(cmd: &SkinCmd) -> anyhow::Result<IpcRequest> {
    Ok(match cmd {
        SkinCmd::List => IpcRequest::SkinList,
        SkinCmd::Import { file } => IpcRequest::SkinImport {
            bytes: std::fs::read(file)
                .map_err(|e| anyhow::anyhow!("{} unreadable: {e}", file.display()))?,
        },
        SkinCmd::Select { id } => IpcRequest::SkinSelect { id: id.clone() },
        SkinCmd::Remove { id } => IpcRequest::SkinRemove { id: id.clone() },
        SkinCmd::SetLogin { id } => IpcRequest::SkinSetLogin { id: id.clone() },
    })
}
