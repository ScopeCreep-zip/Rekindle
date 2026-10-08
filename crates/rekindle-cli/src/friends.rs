//! Friend commands: add, accept, reject, remove, list, requests.

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::FriendCmd;
use crate::helpers;
use crate::output::OutputMode;
use crate::output::{format, table};
use rekindle_client::DaemonClient;

pub async fn dispatch(
    cmd: &FriendCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        FriendCmd::Add { target, message } => {
            let value = client
                .request_ok(IpcRequest::FriendAdd {
                    target: target.clone(),
                    message: message.clone().unwrap_or_default(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        FriendCmd::Accept { request_id } => {
            let value = client
                .request_ok(IpcRequest::FriendAccept {
                    public_key: request_id.clone(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        FriendCmd::Reject { request_id } => {
            let value = client
                .request_ok(IpcRequest::FriendReject {
                    public_key: request_id.clone(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        FriendCmd::Remove { friend, force } => {
            if !helpers::confirm_unless_forced(*force, &format!("Remove friend '{friend}'?"))? {
                return format::print_text("Cancelled.");
            }
            let value = client
                .request_ok(IpcRequest::FriendRemove {
                    public_key: friend.clone(),
                })
                .await?;
            format::print_structured(&value, mode)
        }
        FriendCmd::List { .. } => {
            let value = client.request_ok(IpcRequest::FriendList).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|f| {
                            vec![
                                f.get("display_name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                f.get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("offline")
                                    .to_string(),
                                rekindle_client::fmt::abbreviate_key(
                                    f.get("public_key").and_then(|v| v.as_str()).unwrap_or("?"),
                                ),
                                f.get("has_route")
                                    .and_then(serde_json::Value::as_bool)
                                    .map_or("no".into(), |b| {
                                        if b {
                                            "yes".into()
                                        } else {
                                            "no".into()
                                        }
                                    }),
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            table::print_table(&["Name", "Status", "Key", "Route"], &rows, mode)
        }
        FriendCmd::Requests => {
            let value = client.request_ok(IpcRequest::FriendRequests).await?;
            format::print_structured(&value, mode)
        }
        FriendCmd::Block { .. } | FriendCmd::Unblock { .. } => {
            Err(crate::identity::unimplemented("friend block/unblock"))
        }
    }
}
