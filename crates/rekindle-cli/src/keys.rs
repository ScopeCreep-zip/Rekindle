//! Key management commands: MEK list/rotate/request, prekey status/replenish, inspect.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::KeyCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

pub async fn dispatch(cmd: &KeyCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        KeyCmd::Mek(sub) => match sub {
            crate::cli::MekCmd::List { community } => {
                let value = helpers::send(
                    client,
                    IpcRequest::MekList {
                        community: community.clone(),
                    },
                )
                .await?;
                format::print_structured(&value, mode)
            }
            crate::cli::MekCmd::Rotate { community, channel } => {
                let value = helpers::send(
                    client,
                    IpcRequest::MekRotate {
                        community: community.clone(),
                        channel: channel.clone(),
                    },
                )
                .await?;
                format::print_structured(&value, mode)
            }
            crate::cli::MekCmd::Request { community, channel } => {
                let value = helpers::send(
                    client,
                    IpcRequest::MekRequest {
                        community: community.clone(),
                        channel: channel.clone(),
                        generation: 0, // daemon resolves latest needed generation
                    },
                )
                .await?;
                format::print_structured(&value, mode)
            }
        },
        KeyCmd::Prekeys(sub) => match sub {
            crate::cli::PrekeyCmd::Status => {
                let value = helpers::send(client, IpcRequest::Status).await?;
                // Extract crypto checks from the full status snapshot
                if let Ok(snapshot) =
                    serde_json::from_value::<rekindle_types::display::StatusSnapshot>(value.clone())
                {
                    let crypto_checks: Vec<_> = snapshot
                        .checks
                        .into_iter()
                        .filter(|c| c.category == "crypto")
                        .collect();
                    format::print_doctor_checks(&crypto_checks, mode, false)
                } else {
                    format::print_structured(&value, mode)
                }
            }
            crate::cli::PrekeyCmd::Replenish => {
                let value = helpers::send(client, IpcRequest::PrekeyReplenish).await?;
                format::print_structured(&value, mode)
            }
        },
        KeyCmd::Inspect { community } => {
            let value = helpers::send(
                client,
                IpcRequest::MekList {
                    community: community.clone(),
                },
            )
            .await?;
            format::print_structured(&value, mode)
        }
    }
}
