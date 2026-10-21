//! Group DMs (plan E2.6): the creator's name and roster bounds, and the
//! 16-byte group id, checked before any operation runs.

use std::collections::HashSet;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::group_dm::{MAX_MEMBERS, MAX_NAME_CHARS, MIN_MEMBERS};

use super::{page, public_key, Checked};

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
fn create(name: &str, members: &[String]) -> Checked {
    let chars = name.chars().count();
    if name.trim().is_empty() || chars > MAX_NAME_CHARS {
        return Err(IpcResponse::error(
            400,
            format!("invalid group name: must be 1-{MAX_NAME_CHARS} characters, got {chars}"),
        ));
    }
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
        IpcRequest::GroupDmCreate { name, members } => create(name, members),
        IpcRequest::GroupDmAccept { group_id: id }
        | IpcRequest::GroupDmDecline { group_id: id }
        | IpcRequest::GroupDmLeave { group_id: id } => group_id(id),
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
        assert!(create("raid", &two).is_ok());
        assert!(create("  ", &two).is_err());
        assert!(create(&"x".repeat(MAX_NAME_CHARS + 1), &two).is_err());
        assert!(create("raid", &[key('a')]).is_err());
        assert!(create("raid", &vec![key('a'); 2]).is_err());
        let eight: Vec<String> = "abcdef01".chars().map(key).collect();
        assert!(create("raid", &eight).is_err());
        assert!(create("raid", &eight[..7]).is_ok());
    }

    #[test]
    fn group_ids_are_sixteen_bytes_of_lowercase_hex() {
        assert!(group_id(&"0f".repeat(16)).is_ok());
        assert!(group_id(&"0F".repeat(16)).is_err());
        assert!(group_id(&"0f".repeat(15)).is_err());
    }
}
