//! Friend commands (plan E2.1): requests, the list, removal and blocks,
//! nicknames, invite links, groups and session resets. The host reviews
//! what needs it (`helpers::send`).

use rekindle_ipc::protocol::IpcRequest;

use crate::cli::{FriendCmd, FriendInviteCmd, GroupCmd};
use crate::helpers;
use crate::output::OutputMode;
use crate::output::{format, table};
use rekindle_client::DaemonClient;

pub async fn dispatch(
    cmd: &FriendCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        FriendCmd::Add { target, message } => IpcRequest::FriendAdd {
            public_key: target.clone(),
            display_name: rekindle_client::fmt::abbreviate_key(target),
            message: message.clone().unwrap_or_default(),
        },
        FriendCmd::Accept { request_id } => IpcRequest::FriendAccept {
            display_name: requester_name(client, request_id).await?,
            public_key: request_id.clone(),
        },
        FriendCmd::Reject { request_id } => IpcRequest::FriendReject {
            public_key: request_id.clone(),
        },
        FriendCmd::Cancel { friend } => IpcRequest::FriendCancelRequest {
            public_key: friend.clone(),
        },
        FriendCmd::Remove { friend } => IpcRequest::FriendRemove {
            public_key: friend.clone(),
        },
        FriendCmd::List { status, .. } => return list(client, status.as_deref(), mode).await,
        FriendCmd::Requests => IpcRequest::FriendRequests,
        FriendCmd::Block { friend } => IpcRequest::FriendBlock {
            public_key: friend.clone(),
            display_name: None,
        },
        FriendCmd::Unblock { friend } => IpcRequest::FriendUnblock {
            public_key: friend.clone(),
        },
        FriendCmd::Blocked => IpcRequest::FriendBlockedList,
        FriendCmd::Nickname { friend, name } => IpcRequest::FriendSetNickname {
            public_key: friend.clone(),
            nickname: name.clone(),
        },
        FriendCmd::Invite(invite) => invite_request(invite),
        FriendCmd::Group(group) => group_request(group),
        FriendCmd::ResetSession { friend } => IpcRequest::SessionResetStart {
            public_key: friend.clone(),
        },
    };
    let value = helpers::send(client, request).await?;
    format::print_structured(&value, mode)
}

fn invite_request(cmd: &FriendInviteCmd) -> IpcRequest {
    match cmd {
        FriendInviteCmd::Create => IpcRequest::FriendInviteCreate,
        FriendInviteCmd::List => IpcRequest::FriendInviteList,
        FriendInviteCmd::Cancel { invite_id } => IpcRequest::FriendInviteCancel {
            invite_id: invite_id.clone(),
        },
        FriendInviteCmd::Redeem { link } => IpcRequest::FriendAddFromInvite {
            invite: link.clone(),
        },
    }
}

fn group_request(cmd: &GroupCmd) -> IpcRequest {
    match cmd {
        GroupCmd::Create { name } => IpcRequest::FriendGroupCreate { name: name.clone() },
        GroupCmd::Rename { group_id, name } => IpcRequest::FriendGroupRename {
            group_id: *group_id,
            name: name.clone(),
        },
        GroupCmd::Delete { group_id } => IpcRequest::FriendGroupDelete {
            group_id: *group_id,
        },
        GroupCmd::Move { friend, group_id } => IpcRequest::FriendGroupMove {
            public_key: friend.clone(),
            group_id: *group_id,
        },
    }
}

/// The name a pending request arrived with, for accepting it; its key,
/// abbreviated, when no request names it.
async fn requester_name(client: &DaemonClient, public_key: &str) -> anyhow::Result<String> {
    let requests = helpers::send(client, IpcRequest::FriendRequests).await?;
    let name = requests.as_array().and_then(|list| {
        list.iter()
            .find(|r| r.get("publicKey").and_then(|v| v.as_str()) == Some(public_key))
            .and_then(|r| r.get("displayName").and_then(|v| v.as_str()))
            .map(str::to_string)
    });
    Ok(name.unwrap_or_else(|| rekindle_client::fmt::abbreviate_key(public_key)))
}

/// `friend list`: the buddy list, filtered by status.
async fn list(client: &DaemonClient, status: Option<&str>, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(client, IpcRequest::FriendList).await?;
    if mode.is_structured() {
        return format::print_structured(&value, mode);
    }
    let text = |f: &serde_json::Value, key: &str| {
        f.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let rows = value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|f| status.is_none_or(|s| s == "all" || text(f, "status") == s))
                .map(|f| {
                    let name = f
                        .get("nickname")
                        .and_then(|v| v.as_str())
                        .map_or_else(|| text(f, "displayName"), str::to_string);
                    vec![
                        rekindle_utils::text::sanitize_for_display(&name),
                        text(f, "status"),
                        rekindle_client::fmt::abbreviate_key(&text(f, "publicKey")),
                        text(f, "friendshipState"),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    table::print_table(&["Name", "Status", "Key", "Friendship"], &rows, mode)
}
