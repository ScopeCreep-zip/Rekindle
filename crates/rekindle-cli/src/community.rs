//! Community commands: create, join, leave, list, info.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};

use crate::cli::CommunityCmd;
use crate::helpers;
use crate::output::OutputMode;
use crate::output::{format, table};
use rekindle_client::DaemonClient;

pub async fn dispatch(
    cmd: &CommunityCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        CommunityCmd::Create {
            name,
            description,
            approval_required,
            ..
        } => {
            let validated_name = helpers::validate_name(name, "Community")?;
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityCreate {
                        name: validated_name,
                        description: description.clone().unwrap_or_default(),
                        approval_required: *approval_required,
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Join { invite, .. } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityJoin {
                        invite: invite.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Leave { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityLeave {
                        governance_key: community.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::List => {
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
                                    c.get("governanceKey")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("?"),
                                ),
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            table::print_table(&["Name", "Members", "Channels", "Key"], &rows, mode)
        }
        CommunityCmd::Info { community, full } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityInfo {
                        governance_key: community.clone(),
                    },
                )),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let pairs = vec![
                (
                    "Name",
                    value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                ),
                (
                    "Description",
                    value
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                (
                    "Members",
                    value
                        .get("member_count")
                        .and_then(serde_json::Value::as_u64)
                        .map_or("?".into(), |n| n.to_string()),
                ),
                (
                    "Channels",
                    value
                        .get("channels")
                        .and_then(|v| v.as_array())
                        .map_or("0".into(), |a| a.len().to_string()),
                ),
                (
                    "Our Pseudonym",
                    value
                        .get("our_pseudonym")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                ),
                (
                    "Governance Key",
                    value
                        .get("governance_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                ),
            ];
            table::print_kv_table(&pairs, mode)?;
            if *full {
                print_full_info(&value, mode)?;
            }
            Ok(())
        }
        CommunityCmd::Approve { community, member } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityApprove {
                        governance_key: community.clone(),
                        member_pseudonym: member.clone(),
                    },
                )),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let slot = value
                .get("slot")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            format::print_text(&format!("Approved {member} (slot {slot})"))
        }
        CommunityCmd::Reject {
            community,
            member,
            reason,
        } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityReject {
                        governance_key: community.clone(),
                        member_pseudonym: member.clone(),
                        reason: reason.clone().unwrap_or_default(),
                    },
                )),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Rejected {member}"))
        }
        CommunityCmd::Pending { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityPendingMembers {
                        governance_key: community.clone(),
                    },
                )),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let rows = value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|p| {
                            vec![
                                rekindle_client::fmt::abbreviate_key(
                                    p.get("requester_pseudonym_hex")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("?"),
                                ),
                                p.get("display_name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                p.get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("pending")
                                    .to_string(),
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            table::print_table(&["Pseudonym", "Name", "Status"], &rows, mode)
        }
        CommunityCmd::Transfer {
            community,
            new_owner,
        } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityTransferOwnership {
                        governance_key: community.clone(),
                        new_owner_pseudonym: new_owner.clone(),
                    },
                )),
            )
            .await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            format::print_text(&format!("Ownership transferred to {new_owner}"))
        }
        CommunityCmd::Invite(sub) => crate::governance::dispatch_invite(sub, client, mode).await,
    }
}

/// `community info --full`: the channel list and the roles with their
/// permission bits, from the `CommunityDetail` answer.
fn print_full_info(value: &serde_json::Value, mode: OutputMode) -> anyhow::Result<()> {
    use rekindle_types::display::{ChannelOverviewDisplay, RoleDisplay};
    let channels: Vec<ChannelOverviewDisplay> =
        serde_json::from_value(value.get("channels").cloned().unwrap_or_default())?;
    let roles: Vec<RoleDisplay> =
        serde_json::from_value(value.get("roles").cloned().unwrap_or_default())?;
    format::print_text("")?;
    table::print_table(
        &["Channel", "Kind", "Topic", "Key gen", "ID"],
        &channel_rows(&channels),
        mode,
    )?;
    format::print_text("")?;
    table::print_table(
        &["Role", "Position", "Permissions", "ID"],
        &role_rows(&roles),
        mode,
    )
}

fn channel_rows(channels: &[rekindle_types::display::ChannelOverviewDisplay]) -> Vec<Vec<String>> {
    let mut sorted: Vec<_> = channels.iter().collect();
    sorted.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });
    sorted
        .into_iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.kind.clone(),
                c.topic.clone(),
                c.mek_generation.to_string(),
                rekindle_client::fmt::abbreviate_key(&c.id),
            ]
        })
        .collect()
}

fn role_rows(roles: &[rekindle_types::display::RoleDisplay]) -> Vec<Vec<String>> {
    let mut sorted: Vec<_> = roles.iter().collect();
    sorted.sort_by(|a, b| b.position.cmp(&a.position).then_with(|| a.id.cmp(&b.id)));
    sorted
        .into_iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.position.to_string(),
                format!("{:#x}", r.permissions),
                r.id.to_string(),
            ]
        })
        .collect()
}
