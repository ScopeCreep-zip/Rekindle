//! Community commands: create, join, leave, list, info.

use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, SettingsRequest};

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
            co_creators,
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
                        co_creators: co_creators.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::CreatorRemove { anchor, identity } => {
            let (anchor, identity) = (anchor.clone(), identity.clone());
            lifecycle(
                client,
                mode,
                LifecycleRequest::CreatorRemove { anchor, identity },
            )
            .await
        }
        CommunityCmd::CreateCancel { anchor } => {
            let anchor = anchor.clone();
            lifecycle(
                client,
                mode,
                LifecycleRequest::CommunityCreateCancel { anchor },
            )
            .await
        }
        CommunityCmd::CreatorAnswer { anchor, decline } => {
            let (anchor, accept) = (anchor.clone(), !*decline);
            lifecycle(
                client,
                mode,
                LifecycleRequest::CreatorInviteAnswer { anchor, accept },
            )
            .await
        }
        CommunityCmd::Join { invite, .. } => {
            let invite = invite.clone();
            lifecycle(client, mode, LifecycleRequest::CommunityJoin { invite }).await
        }
        CommunityCmd::Leave { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityLeave {
                        anchor: community.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::SetName { community, name } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Settings(
                    SettingsRequest::CommunitySetDisplayName {
                        community: community.clone(),
                        display_name: name.clone(),
                    },
                )),
            )
            .await?;
            format::print_structured(&value, mode)
        }
        CommunityCmd::Forget { community } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityForget {
                        anchor: community.clone(),
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
        CommunityCmd::Info { community, full } => {
            let value = helpers::send(
                client,
                IpcRequest::Community(CommunityRequest::Lifecycle(
                    LifecycleRequest::CommunityInfo {
                        anchor: community.clone(),
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
                    "Anchor Key",
                    value
                        .get("anchor")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                ),
            ];
            let mut pairs = pairs;
            pairs.push(("Standing", removal_text(value.get("removed"))));
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
                        anchor: community.clone(),
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
                        anchor: community.clone(),
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
                        anchor: community.clone(),
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
        CommunityCmd::PublishSuccessor { .. }
        | CommunityCmd::FollowSuccessor { .. }
        | CommunityCmd::Successors { .. } => successor(cmd, client, mode).await,
        CommunityCmd::Transfer { .. }
        | CommunityCmd::WithdrawTransfer { .. }
        | CommunityCmd::AcceptTransfer { .. }
        | CommunityCmd::AnswerLink { .. } => transfer(cmd, client, mode).await,
        CommunityCmd::RetireDevice { .. }
        | CommunityCmd::ReadmitDevice { .. }
        | CommunityCmd::EnrolDevice { .. } => device(cmd, client, mode).await,
        CommunityCmd::Invite(sub) => crate::governance::dispatch_invite(sub, client, mode).await,
        CommunityCmd::Watch { community } => {
            crate::community_watch::watch(client, community.as_deref(), mode).await
        }
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
                r.id.clone(),
            ]
        })
        .collect()
}

/// A community's standing as list and info show it (plan E3.2 S9 R3-5):
/// "member", or "removed"/"banned" with the remover's card name or short
/// pseudonym when the merge names one.
fn removal_text(removed: Option<&serde_json::Value>) -> String {
    let Some(removed) = removed.filter(|r| !r.is_null()) else {
        return "member".into();
    };
    let what = if removed.get("banned").and_then(serde_json::Value::as_bool) == Some(true) {
        "banned"
    } else {
        "removed"
    };
    let by = (removed.get("byName").and_then(|v| v.as_str()))
        .map(str::to_owned)
        .or_else(|| {
            (removed.get("by").and_then(|v| v.as_str())).map(rekindle_client::fmt::abbreviate_key)
        });
    let standing = match by {
        Some(by) => format!("{what} by {by}"),
        None => what.into(),
    };
    match (removed.get("reason").and_then(|v| v.as_str())).filter(|r| !r.is_empty()) {
        Some(reason) => format!("{standing}: {reason}"),
        None => standing,
    }
}

/// The successor commands (plan E3.2 S7s.4): publish this community's
/// successor, follow one, or list the offers kept for the snapshot.
async fn successor(
    cmd: &CommunityCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        CommunityCmd::PublishSuccessor {
            community,
            continuing,
        } => LifecycleRequest::CommunityPublishSuccessor {
            anchor: community.clone(),
            continuing: (!continuing.is_empty()).then(|| continuing.clone()),
        },
        CommunityCmd::FollowSuccessor {
            community,
            new_anchor,
        } => LifecycleRequest::CommunityFollowSuccessor {
            anchor: community.clone(),
            new_anchor: new_anchor.clone(),
        },
        CommunityCmd::Successors { community } => LifecycleRequest::CommunitySuccessors {
            anchor: community.clone(),
        },
        _ => anyhow::bail!("not a successor command"),
    };
    lifecycle(client, mode, request).await
}

/// The transfer commands (plan E3.2 S7s.4): offer, withdraw, accept.
async fn transfer(
    cmd: &CommunityCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        CommunityCmd::Transfer {
            community,
            new_owner,
        } => LifecycleRequest::CommunityOfferTransfer {
            anchor: community.clone(),
            transfer_to: new_owner.clone(),
        },
        CommunityCmd::WithdrawTransfer { community } => {
            LifecycleRequest::CommunityWithdrawTransfer {
                anchor: community.clone(),
            }
        }
        CommunityCmd::AcceptTransfer { community } => LifecycleRequest::CommunityAcceptTransfer {
            anchor: community.clone(),
        },
        CommunityCmd::AnswerLink { community, request } => {
            LifecycleRequest::CommunityAnswerSuccessorLink {
                anchor: community.clone(),
                request: request.clone(),
            }
        }
        _ => anyhow::bail!("not a transfer command"),
    };
    lifecycle(client, mode, request).await
}

/// The device-key commands (ADR 0022, plan E3.2 S7d-keys K5): retire one
/// of our device keys, or co-sign a member's fork-answering key back in.
async fn device(cmd: &CommunityCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    let request = match cmd {
        CommunityCmd::EnrolDevice { community } => LifecycleRequest::CommunityDeviceEnrol {
            anchor: community.clone(),
        },
        CommunityCmd::RetireDevice {
            community,
            device_key,
        } => LifecycleRequest::CommunityDeviceRetire {
            anchor: community.clone(),
            device_key: device_key.clone(),
        },
        CommunityCmd::ReadmitDevice {
            community,
            member,
            device_key,
        } => LifecycleRequest::CommunityDeviceReadmit {
            anchor: community.clone(),
            member: member.clone(),
            device_key: device_key.clone(),
        },
        _ => anyhow::bail!("not a device command"),
    };
    lifecycle(client, mode, request).await
}

/// Send a lifecycle `request` and print its answer (the creator-set
/// round's commands, plan E3.2 S7s.3).
async fn lifecycle(
    client: &DaemonClient,
    mode: OutputMode,
    request: LifecycleRequest,
) -> anyhow::Result<()> {
    let value = helpers::send(
        client,
        IpcRequest::Community(CommunityRequest::Lifecycle(request)),
    )
    .await?;
    format::print_structured(&value, mode)
}

#[cfg(test)]
mod removal_tests {
    use super::removal_text;

    #[test]
    fn standing_names_who_removed_us() {
        assert_eq!(removal_text(None), "member");
        assert_eq!(removal_text(Some(&serde_json::Value::Null)), "member");
        let kicked = serde_json::json!({ "banned": false, "by": "ab".repeat(32), "byName": null });
        assert!(
            removal_text(Some(&kicked)).starts_with("removed by "),
            "{}",
            removal_text(Some(&kicked))
        );
        let named = serde_json::json!({ "banned": true, "by": "ab".repeat(32), "byName": "Ann" });
        assert_eq!(removal_text(Some(&named)), "banned by Ann");
        let why = serde_json::json!({ "banned": false, "by": "ab".repeat(32), "byName": "Ann", "reason": "spam" });
        assert_eq!(removal_text(Some(&why)), "removed by Ann: spam");
        let nobody = serde_json::json!({ "banned": false, "by": null, "byName": null });
        assert_eq!(removal_text(Some(&nobody)), "removed");
    }
}
