//! Presence commands: set status, game presence.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::PresenceCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(
    cmd: &PresenceCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        PresenceCmd::Set {
            status, message, ..
        } => {
            let value = client
                .request_ok(IpcRequest::PresenceSet {
                    status: status.clone(),
                    message: message.clone(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        PresenceCmd::Watch { .. } => {
            let value = client.request_ok(IpcRequest::Status).await?;
            format::print_structured(&value, mode)
        }
    }
}
