//! Group DMs (plan E2.6): the creator's name and roster bounds, and the
//! 16-byte group id, and a message body under the direct send's caps,
//! checked before any operation runs.

use std::collections::HashSet;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::group_dm::{MAX_MEMBERS, MAX_NAME_CHARS, MIN_MEMBERS};

use super::{message_body, page, public_key, Checked};

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
        IpcRequest::GroupDmCreate {
            name,
            members,
            origin_peer,
        } => create(name, members, origin_peer.as_deref()),
        IpcRequest::GroupDmAccept { group_id: id }
        | IpcRequest::GroupDmDecline { group_id: id }
        | IpcRequest::GroupDmLeave { group_id: id }
        | IpcRequest::GroupDmMarkRead { group_id: id }
        | IpcRequest::GroupDmTyping {
            group_id: id,
            typing: _,
        } => group_id(id),
        IpcRequest::GroupDmInvite {
            group_id: id,
            members,
        } => invite(id, members),
        IpcRequest::GroupDmRename { group_id: id, name } => {
            group_id(id).and_then(|()| group_name(name))
        }
        IpcRequest::GroupDmSend { group_id: id, body } => {
            group_id(id).and_then(|()| message_body(body))
        }
        IpcRequest::DisappearingSet {
            conversation,
            seconds,
        } => disappearing(conversation, *seconds),
        IpcRequest::DisappearingGet { conversation } => disappearing(conversation, 0),
        IpcRequest::GroupDmList => Ok(()),
        IpcRequest::GroupDmMessages {
            group_id: id,
            limit,
            before: _,
        } => group_id(id).and_then(|()| page(*limit)),
        other => unreachable!("{} is not a group-DM request", other.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(digit: char) -> String {
        std::iter::repeat_n(digit, 64).collect()
    }

    #[test]
    fn create_bounds_name_and_members() {
        let two = vec![key('a'), key('b')];
        assert!(create("raid", &two, None).is_ok());
        assert!(create("  ", &two, None).is_err());
        assert!(create(&"x".repeat(MAX_NAME_CHARS + 1), &two, None).is_err());
        assert!(create("raid", &[key('a')], None).is_err());
        assert!(create("raid", &vec![key('a'); 2], None).is_err());
        let eight: Vec<String> = "abcdef01".chars().map(key).collect();
        assert!(create("raid", &eight, None).is_err());
        assert!(create("raid", &eight[..7], None).is_ok());
        assert!(create("raid", &two, Some(&key('a'))).is_ok());
        assert!(create("raid", &two, Some(&key('c'))).is_err());
    }

    #[test]
    fn a_rename_names_a_group_within_the_name_bounds() {
        let rename = |group_id: String, name: &str| {
            check(&IpcRequest::GroupDmRename {
                group_id,
                name: name.to_owned(),
            })
        };
        let g = "0f".repeat(16);
        assert!(rename(g.clone(), "scrim night").is_ok());
        assert!(rename(g.clone(), &"x".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(rename(g.clone(), &"x".repeat(MAX_NAME_CHARS + 1)).is_err());
        assert!(rename(g, "   ").is_err());
        assert!(rename("0F".repeat(16), "scrim night").is_err());
    }

    #[test]
    fn an_invite_names_a_group_and_one_to_seven_distinct_keys() {
        let g = "0f".repeat(16);
        let seven: Vec<String> = "abcdef0".chars().map(key).collect();
        assert!(invite(&g, &seven[..1]).is_ok());
        assert!(invite(&g, &seven).is_ok());
        let mut eight = seven.clone();
        eight.push(key('1'));
        assert!(invite(&g, &eight).is_err());
        assert!(invite(&g, &[]).is_err());
        assert!(invite(&g, &vec![key('a'); 2]).is_err());
        assert!(invite(&g, &["nope".to_owned()]).is_err());
        assert!(invite(&"0F".repeat(16), &seven[..1]).is_err());
    }

    #[test]
    fn group_ids_are_sixteen_bytes_of_lowercase_hex() {
        assert!(group_id(&"0f".repeat(16)).is_ok());
        assert!(group_id(&"0F".repeat(16)).is_err());
        assert!(group_id(&"0f".repeat(15)).is_err());
    }
}
