//! `rekindle transfer …` (plan E7.2 G12), one request each.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{IpcRequest, TransferRequest};
use rekindle_types::dto::transfer::TransferTarget;

use crate::cli::TransferCmd;
use crate::helpers;
use crate::output::{format, OutputMode};

pub async fn dispatch(
    cmd: &TransferCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let value = helpers::send(client, request(cmd)?).await?;
    format::print_structured(&value, mode)
}

/// The request `cmd` sends.
pub fn request(cmd: &TransferCmd) -> anyhow::Result<IpcRequest> {
    Ok(IpcRequest::Transfer(match cmd {
        TransferCmd::List => TransferRequest::TransferList,
        TransferCmd::Start {
            attachment_id,
            community,
            channel,
            dest,
        } => TransferRequest::TransferStart {
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
        TransferCmd::Stop { attachment_id } => TransferRequest::TransferStop {
            attachment_id: attachment_id.clone(),
        },
        TransferCmd::Open { attachment_id } => TransferRequest::TransferOpen {
            attachment_id: attachment_id.clone(),
        },
        TransferCmd::Remove {
            attachment_id,
            delete_file,
        } => TransferRequest::TransferRemove {
            attachment_id: attachment_id.clone(),
            delete_file: *delete_file,
        },
    }))
}
