//! Presence commands: set status, game presence.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::PresenceCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

pub async fn dispatch(
    cmd: &PresenceCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        PresenceCmd::Set {
            status, message, ..
        } => {
            let value = helpers::send(
                client,
                IpcRequest::PresenceSet {
                    status: status.clone(),
                    message: message.clone(),
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
        PresenceCmd::Watch { .. } => {
            let value = helpers::send(client, IpcRequest::Status).await?;
            format::print_structured(&value, mode)
        }
    }
}
