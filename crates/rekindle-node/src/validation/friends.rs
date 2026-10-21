//! Friends, direct messages and session resets (plan E2.1).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::key_format::{MAX_INVITE_ID_LEN, MAX_NOTE_LEN};

use super::{message_body, name, opt, page, public_key, text, Checked};

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
        IpcRequest::FriendAddFromCode { code, message } => {
            rekindle_types::friend_code::FriendCode::parse(code)
                .map_err(|e| IpcResponse::error(400, e.to_string()))?;
            text(message, MAX_NOTE_LEN, "message")
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
        | IpcRequest::FriendAccept { public_key: key }
        | IpcRequest::FriendBlock { public_key: key }
        | IpcRequest::FriendReject { public_key: key }
        | IpcRequest::FriendRemove { public_key: key }
        | IpcRequest::FriendCancelRequest { public_key: key }
        | IpcRequest::FriendRetryHandshake { public_key: key }
        | IpcRequest::FriendUnblock { public_key: key }
        | IpcRequest::SessionResetStart { public_key: key }
        | IpcRequest::SessionResetAccept { public_key: key }
        | IpcRequest::IdentityRotationAccept { public_key: key }
        | IpcRequest::IdentityRotationDecline { public_key: key }
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
        | IpcRequest::DmMarkRead {
            peer_key,
            rendered_through: None,
        } => public_key(peer_key, "peer key"),
        IpcRequest::DmMarkRead {
            peer_key,
            rendered_through: Some(id),
        } => public_key(peer_key, "peer key").and_then(|()| client_id(id)),
        IpcRequest::DmResend { message_id } => client_id(message_id),
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
