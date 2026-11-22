//! `rekindle skin …` (plan E7.7.2), one request each.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::SkinCmd;

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
        SkinCmd::Keep { id, revert } => IpcRequest::SkinKeep {
            id: id.clone(),
            keep: !revert,
        },
    })
}
