//! `rekindle transfer …` (plan E7.2 G12), one request each.

use rekindle_ipc::protocol::{IpcRequest, TransferRequest};
use rekindle_types::dto::transfer::TransferTarget;

use crate::cli::TransferCmd;

/// The request `cmd` sends.
pub fn request(cmd: &TransferCmd) -> anyhow::Result<IpcRequest> {
    Ok(IpcRequest::Transfer(match cmd {
        TransferCmd::List => TransferRequest::List,
        TransferCmd::Start {
            attachment_id,
            community,
            channel,
            dest,
        } => TransferRequest::Start {
            attachment_id: attachment_id.clone(),
            target: TransferTarget::Channel {
                community: community.clone(),
                channel_id: channel.clone(),
            },
            dest: dest
                .as_ref()
                .map(|d| {
                    d.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow::anyhow!("{} is not UTF-8", d.display()))
                })
                .transpose()?,
        },
        TransferCmd::Stop { attachment_id } => TransferRequest::Stop {
            attachment_id: attachment_id.clone(),
        },
        TransferCmd::Open { attachment_id } => TransferRequest::Open {
            attachment_id: attachment_id.clone(),
        },
        TransferCmd::Remove {
            attachment_id,
            delete_file,
        } => TransferRequest::Remove {
            attachment_id: attachment_id.clone(),
            delete_file: *delete_file,
        },
    }))
}
