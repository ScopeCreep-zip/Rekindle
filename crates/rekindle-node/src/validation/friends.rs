//! Friends, direct messages and session resets (plan E2.1).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::key_format::MAX_NOTE_LEN;

use super::{message_body, name, opt, page, public_key, text, Checked};

/// A peer's display name as a request carries it (theirs, or what the user
/// typed for them).
fn peer_name(s: &str) -> Checked {
    text(
        s,
        rekindle_types::presence::limits::MAX_DISPLAY_NAME_LEN,
        "display name",
    )
}

/// The longest invite id: a hyphenated UUID is 36.
const MAX_INVITE_ID_LEN: usize = 64;

/// A friend invite link, bounded before anything decodes it (its signature
/// and recency are checked where it is reviewed).
fn friend_invite(link: &str) -> Checked {
    let max = rekindle_types::invite::MAX_FRIEND_INVITE_URL_LEN;
    if link.is_empty() || link.len() > max {
        return Err(IpcResponse::error(
            400,
            format!("invalid invite: must be 1-{max} bytes, got {}", link.len()),
        ));
    }
    Ok(())
}

/// A message's client id: a hyphenated UUID.
fn client_id(id: &str) -> Checked {
    if id.len() == 36 && uuid::Uuid::parse_str(id).is_ok() {
        Ok(())
    } else {
        Err(IpcResponse::error(400, "invalid message id: want a UUID"))
    }
}

/// Check a friend, DM or session-reset request.
pub(super) fn check(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::FriendAdd {
            public_key: key,
            display_name,
            message,
        } => {
            public_key(key, "public key")?;
            peer_name(display_name)?;
            text(message, MAX_NOTE_LEN, "message")
        }
        IpcRequest::FriendAccept {
            public_key: key,
            display_name,
        } => {
            public_key(key, "public key")?;
            peer_name(display_name)
        }
        IpcRequest::FriendBlock {
            public_key: key,
            display_name,
        } => {
            public_key(key, "public key")?;
            opt(display_name.as_deref(), peer_name)
        }
        IpcRequest::FriendSetNickname {
            public_key: key,
            nickname,
        } => {
            public_key(key, "public key")?;
            opt(nickname.as_deref(), |n| name(n, "nickname"))
        }
        IpcRequest::FriendGroupMove {
            public_key: key,
            group_id: _,
        }
        | IpcRequest::FriendReject { public_key: key }
        | IpcRequest::FriendRemove { public_key: key }
        | IpcRequest::FriendCancelRequest { public_key: key }
        | IpcRequest::FriendUnblock { public_key: key }
        | IpcRequest::SessionResetStart { public_key: key }
        | IpcRequest::SessionResetAccept { public_key: key }
        | IpcRequest::SessionResetDecline { public_key: key } => public_key(key, "public key"),
        IpcRequest::FriendAddFromInvite { invite } => friend_invite(invite),
        IpcRequest::FriendInviteCancel { invite_id } => {
            text(invite_id, MAX_INVITE_ID_LEN, "invite id")
        }
        IpcRequest::FriendGroupCreate { name: group }
        | IpcRequest::FriendGroupRename {
            group_id: _,
            name: group,
        } => name(group, "group"),

        IpcRequest::DmSend {
            peer_key,
            body,
            reply_to,
        } => {
            public_key(peer_key, "peer key")?;
            opt(reply_to.as_deref(), client_id)?;
            message_body(body)
        }
        IpcRequest::DmTyping {
            peer_key,
            typing: _,
        }
        | IpcRequest::DmMarkRead { peer_key } => public_key(peer_key, "peer key"),
        IpcRequest::DmInbox { limit } => page(*limit),
        IpcRequest::DmHistory {
            peer_key,
            limit,
            before: _,
        } => {
            public_key(peer_key, "peer key")?;
            page(*limit)
        }
        other => unreachable!("{} is not a friend or DM request", other.name()),
    }
}
