//! Friend commands (plan E2.1): requests, the list, removal and blocks,
//! nicknames, invite links, groups, session resets, and safety numbers and
//! verification (plan E2.3). The host reviews what needs it
//! (`helpers::send`).

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::friends::SafetyNumberView;

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
        FriendCmd::Add { code, message } => IpcRequest::FriendAddFromCode {
            code: code.clone(),
            message: message.clone().unwrap_or_default(),
        },
        FriendCmd::Accept { request_id } => IpcRequest::FriendAccept {
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
        FriendCmd::SafetyNumber { friend } => return safety_number(client, friend, mode).await,
        FriendCmd::Verify { friend } => IpcRequest::IdentityMarkVerified {
            peer: friend.clone(),
        },
        FriendCmd::Unverify { friend } => IpcRequest::IdentityClearVerification {
            peer: friend.clone(),
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

/// `friend safety-number`: the 60 digits in twelve groups of five (four to
/// a line, as Signal shows them), the QR code, and the verified state.
async fn safety_number(client: &DaemonClient, peer: &str, mode: OutputMode) -> anyhow::Result<()> {
    let value = helpers::send(
        client,
        IpcRequest::IdentitySafetyNumber {
            peer: peer.to_string(),
        },
    )
    .await?;
    if mode.is_structured() {
        return format::print_structured(&value, mode);
    }
    let view: SafetyNumberView = serde_json::from_value(value)?;
    let groups: Vec<&str> = view
        .digits
        .as_bytes()
        .chunks(5)
        .filter_map(|group| std::str::from_utf8(group).ok())
        .collect();
    for line in groups.chunks(4) {
        format::print_text(&line.join(" "))?;
    }
    format::print_text("")?;
    format::print_text(&qr_text(&view.qr_rows))?;
    let state = if view.changed {
        "their key changed since you verified it: compare the new number"
    } else if view.verified {
        "verified"
    } else {
        "not verified"
    };
    format::print_text(state)
}

/// QR rows as terminal text: two rows per line in half blocks, dark on a
/// light quiet zone of four modules, as a scanner needs.
fn qr_text(rows: &[String]) -> String {
    const QUIET: usize = 4;
    let width = rows.first().map_or(0, String::len) + 2 * QUIET;
    let dark = |y: usize, x: usize| {
        y.checked_sub(QUIET)
            .and_then(|y| rows.get(y))
            .zip(x.checked_sub(QUIET))
            .and_then(|(row, x)| row.as_bytes().get(x))
            == Some(&b'1')
    };
    let height = rows.len() + 2 * QUIET;
    (0..height)
        .step_by(2)
        .map(|y| {
            (0..width)
                .map(|x| match (dark(y, x), dark(y + 1, x)) {
                    (true, true) => ' ',
                    (true, false) => '\u{2584}',
                    (false, true) => '\u{2580}',
                    (false, false) => '\u{2588}',
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::qr_text;

    #[test]
    fn a_qr_is_drawn_light_around_dark() {
        let text = qr_text(&["10".into(), "01".into()]);
        let lines: Vec<&str> = text.lines().collect();
        // (2 rows + 8 quiet) / 2 lines, (2 + 8) columns each.
        assert_eq!(lines.len(), 5);
        assert!(lines.iter().all(|l| l.chars().count() == 10));
        assert!(lines[0].chars().all(|c| c == '\u{2588}'));
        assert_eq!(lines[2].chars().nth(4), Some('\u{2584}'));
        assert_eq!(lines[2].chars().nth(5), Some('\u{2580}'));
    }
}
