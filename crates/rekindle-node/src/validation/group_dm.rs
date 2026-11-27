//! Group DMs (plan E2.6): the creator's name and roster bounds, and the
//! 16-byte group id, and a message body under the direct send's caps,
//! checked before any operation runs.

use std::collections::HashSet;

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, IpcResponse, LobbyRequest};
use rekindle_types::group_dm::{MAX_MEMBERS, MAX_NAME_CHARS, MIN_MEMBERS};

use super::friends::client_id;
use super::{message_body, opt, page, public_key, Checked};

/// A group id: the creator's 16 random bytes, 32 lowercase hex.
fn group_id(id: &str) -> Checked {
    let hex = id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if hex {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            "invalid group id: want 16 bytes as 32 lowercase hex",
        ))
    }
}

/// A new group's name (1-[`MAX_NAME_CHARS`] characters, not blank) and
/// its other members: distinct keys, [`MIN_MEMBERS`]-[`MAX_MEMBERS`] with us.
/// A timer request (plan E7.1): a friend's key or a group's id, and a
/// timer of at most four weeks (Signal's presets or a custom value).
fn disappearing(conversation: &str, seconds: u32) -> Checked {
    if !rekindle_types::disappearing::valid(seconds) {
        return Err(IpcResponse::error(400, "the timer is at most four weeks"));
    }
    if conversation.len() == 32 {
        group_id(conversation)
    } else {
        public_key(conversation, "conversation")
    }
}

/// A group's name: 1-[`MAX_NAME_CHARS`] characters, not blank.
fn group_name(name: &str) -> Checked {
    let chars = name.chars().count();
    if name.trim().is_empty() || chars > MAX_NAME_CHARS {
        return Err(IpcResponse::error(
            400,
            format!("invalid group name: must be 1-{MAX_NAME_CHARS} characters, got {chars}"),
        ));
    }
    Ok(())
}

fn create(name: &str, members: &[String], origin_peer: Option<&str>) -> Checked {
    if origin_peer.is_some_and(|peer| !members.iter().any(|m| m == peer)) {
        return Err(IpcResponse::error(
            400,
            "invalid group: the chat it comes from must be a member's",
        ));
    }
    group_name(name)?;
    let (min, max) = (MIN_MEMBERS - 1, MAX_MEMBERS - 1);
    if members.len() < min || members.len() > max {
        return Err(IpcResponse::error(
            400,
            format!(
                "invalid group: pick {min}-{max} friends, got {}",
                members.len()
            ),
        ));
    }
    distinct_keys(members)
}

/// A roster growth (plan E7.2 G8): a group id and 1-7 distinct friends'
/// keys, since a group holds at most [`MAX_MEMBERS`] with its creator; the
/// operation checks the grown roster against the held one.
fn invite(id: &str, members: &[String]) -> Checked {
    group_id(id)?;
    let max = MAX_MEMBERS - 1;
    if members.is_empty() || members.len() > max {
        return Err(IpcResponse::error(
            400,
            format!(
                "invalid invite: pick 1-{max} friends, got {}",
                members.len()
            ),
        ));
    }
    distinct_keys(members)
}

/// Each a member key, none named twice.
fn distinct_keys(members: &[String]) -> Checked {
    let mut seen = HashSet::with_capacity(members.len());
    for member in members {
        public_key(member, "member key")?;
        if !seen.insert(member.as_str()) {
            return Err(IpcResponse::error(
                400,
                "invalid group: a member is named twice",
            ));
        }
    }
    Ok(())
}

pub(super) fn check(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::GroupDm(GroupDmRequest::Create {
            name,
            members,
            origin_peer,
        }) => create(name, members, origin_peer.as_deref()),
        IpcRequest::GroupDm(
            GroupDmRequest::Accept { group_id: id }
            | GroupDmRequest::Decline { group_id: id }
            | GroupDmRequest::Leave { group_id: id }
            | GroupDmRequest::MarkRead { group_id: id }
            | GroupDmRequest::Typing {
                group_id: id,
                typing: _,
            },
        )
        | IpcRequest::Lobby(
            LobbyRequest::Host { group_id: id } | LobbyRequest::Get { group_id: id },
        ) => group_id(id),
        IpcRequest::GroupDm(GroupDmRequest::Invite {
            group_id: id,
            members,
        }) => invite(id, members),
        IpcRequest::Lobby(
            LobbyRequest::Join {
                group_id: id,
                lobby_id,
            }
            | LobbyRequest::Leave {
                group_id: id,
                lobby_id,
            }
            | LobbyRequest::Close {
                group_id: id,
                lobby_id,
            }
            | LobbyRequest::Launch {
                group_id: id,
                lobby_id,
                server: _,
            },
        ) => group_id(id).and_then(|()| group_id(lobby_id)),
        IpcRequest::GroupDm(GroupDmRequest::Rename { group_id: id, name }) => {
            group_id(id).and_then(|()| group_name(name))
        }
        IpcRequest::GroupDm(GroupDmRequest::Send {
            group_id: id,
            body,
            reply_to,
        }) => {
            group_id(id)?;
            opt(reply_to.as_deref(), client_id)?;
            message_body(body)
        }
        IpcRequest::DisappearingSet {
            conversation,
            seconds,
        } => disappearing(conversation, *seconds),
        IpcRequest::DisappearingGet { conversation } => disappearing(conversation, 0),
        IpcRequest::GroupDm(GroupDmRequest::List) => Ok(()),
        IpcRequest::GroupDm(GroupDmRequest::Messages {
            group_id: id,
            limit,
            before: _,
        }) => group_id(id).and_then(|()| page(*limit)),
        other => unreachable!("{} is not a group-DM request", other.name()),
    }
}

#[cfg(test)]
#[path = "group_dm_tests.rs"]
mod tests;
