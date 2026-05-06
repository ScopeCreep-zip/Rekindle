//! Key management commands: MEK list/rotate/request, prekey status/replenish, inspect.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::KeyCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::DaemonClient;

pub async fn dispatch(cmd: &KeyCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        KeyCmd::Mek(sub) => match sub {
            crate::cli::MekCmd::List { community } => {
                let value = client.request_ok(IpcRequest::MekList { community: community.clone() }).await?;
                format::print_structured(&value, mode)
            }
            crate::cli::MekCmd::Rotate { community, channel } => {
                let value = client.request_ok(IpcRequest::MekRotate {
                    community: community.clone(),
                    channel: channel.clone(),
                }).await?;
                let target = format!("{community}/{channel}");
                helpers::audit_log("mek_rotate", &target, "ok");
                format::print_structured(&value, mode)
            }
            crate::cli::MekCmd::Request { community, channel } => {
                let value = client.request_ok(IpcRequest::MekRequest {
                    community: community.clone(),
                    channel: channel.clone(),
                    generation: 0, // daemon resolves latest needed generation
                }).await?;
                format::print_structured(&value, mode)
            }
        },
        KeyCmd::Prekeys(sub) => match sub {
            crate::cli::PrekeyCmd::Status => {
                let value = client.request_ok(IpcRequest::Status).await?;
                // Extract crypto checks from the full status snapshot
                if let Ok(snapshot) = serde_json::from_value::<rekindle_types::display::StatusSnapshot>(value.clone()) {
                    let crypto_checks: Vec<_> = snapshot.checks.into_iter()
                        .filter(|c| c.category == "crypto")
                        .collect();
                    format::print_doctor_checks(&crypto_checks, mode, false)
                } else {
                    format::print_structured(&value, mode)
                }
            }
            crate::cli::PrekeyCmd::Replenish => {
                let value = client.request_ok(IpcRequest::PrekeyReplenish).await?;
                format::print_structured(&value, mode)
            }
        },
        KeyCmd::Inspect { community } => {
            let value = client.request_ok(IpcRequest::MekList { community: community.clone() }).await?;
            format::print_structured(&value, mode)
        }
    }
}
