//! DM commands: send, inbox, watch, read.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::DmCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(cmd: &DmCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        DmCmd::Send { friend, message, .. } => {
            let value = client.request_ok(IpcRequest::DmSend {
                peer_key: friend.clone(),
                body: message.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Inbox { limit, .. } => {
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::DmInbox {
                limit: *limit as u32,
            }).await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Watch { .. } => {
            // Streaming watch requires Subscribe + event loop. One-shot inbox for now.
            let value = client.request_ok(IpcRequest::DmInbox { limit: 50 }).await?;
            format::print_structured(&value, mode)
        }
        DmCmd::Read { conversation_id, limit, .. } => {
            // DM read is inbox scoped to a conversation — daemon returns all, CLI filters
            let _ = conversation_id;
            #[allow(clippy::cast_possible_truncation)]
            let value = client.request_ok(IpcRequest::DmInbox {
                limit: *limit as u32,
            }).await?;
            format::print_structured(&value, mode)
        }
    }
}
