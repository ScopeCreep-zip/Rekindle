//! The community list served by the Host, rendered for the terminal.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use super::removal_text;
use crate::helpers;
use crate::output::{format, table, OutputMode};

pub(super) async fn dispatch(client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(
        client,
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::CommunityList)),
    )
    .await?;
    if mode.is_structured() {
        return format::print_structured(&value, mode);
    }
    let rows = value
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    vec![
                        c.get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                            .to_string(),
                        c.get("memberCount")
                            .and_then(serde_json::Value::as_u64)
                            .map_or("?".into(), |n| n.to_string()),
                        c.get("channelCount")
                            .and_then(serde_json::Value::as_u64)
                            .map_or("?".into(), |n| n.to_string()),
                        rekindle_client::fmt::abbreviate_key(
                            c.get("anchor").and_then(|v| v.as_str()).unwrap_or("?"),
                        ),
                        removal_text(c.get("removed")),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    table::print_table(
        &["Name", "Members", "Channels", "Key", "Standing"],
        &rows,
        mode,
    )
}
