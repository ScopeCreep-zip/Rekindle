//! Group DMs (plan E2.6): 3-8 members, no governance, sender-owned keys.

use serde::{Deserialize, Serialize};

/// The most members a group DM holds, its creator included.
pub const MAX_MEMBERS: usize = 8;

/// The fewest members a group DM holds, its creator included.
pub const MIN_MEMBERS: usize = 3;

/// The most characters in a group's name.
pub const MAX_NAME_CHARS: usize = 64;

/// One member on a group's signed roster (the creator's `GroupDmInvite`),
/// stored in `group_dms.roster`. Its group chain is not here: each member
/// announces its own over the pairwise session (`GroupDmAccept`,
/// `StreamRecord{group_id}`; plan E2.6, decided 2026-10-21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmRosterMember {
    /// The creator-assigned generation of this admission to the group.
    pub membership_generation: [u8; 16],
    /// The member's identity key, 64 lowercase hex.
    pub identity: String,
    /// The member's profile record, where its prekey record is found
    /// (plan E2.4's B2 pick) by the first sender of a pair.
    pub profile_record_key: String,
    /// The creator's name for the member, signed with the roster: the
    /// creator's own profile name for itself, its friend's name for each
    /// other member. A label a non-friend shows under, never an identity
    /// (threat model: spoofable display fields); the Host marks it as not
    /// a friend's.
    pub display_name: String,
}

/// Why a pending invite's room actions wait: renaming, inviting, calling.
pub const ACCEPT_INVITE_FIRST: &str = "accept the invite first";

/// What we hold of a group DM, as a group call reads it (plan E4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupCallRoster {
    /// A group we are in: its roster members.
    Held(Vec<GroupDmRosterMember>),
    /// An invite to it we have not answered.
    Invited,
    /// Neither a group we hold nor an invite to one.
    Unknown,
}

/// Why `me` may not start a call in the group `roster` describes, or
/// `None` when it may (plan E4.1): the group is ours, we are on its
/// roster, and someone else is too. The group's view serves this as
/// `callRefused` and `start_group_call` refuses with the same words.
#[must_use]
pub fn group_call_refused(roster: &GroupCallRoster, me: &str) -> Option<String> {
    match roster {
        GroupCallRoster::Invited => Some(ACCEPT_INVITE_FIRST.to_owned()),
        GroupCallRoster::Unknown => Some("the group is not in your list".to_owned()),
        GroupCallRoster::Held(members) if !members.iter().any(|member| member.identity == me) => {
            Some("you are not in the group".to_owned())
        }
        GroupCallRoster::Held(members) if members.iter().all(|member| member.identity == me) => {
            Some("no one else is in the group to call".to_owned())
        }
        GroupCallRoster::Held(_) => None,
    }
}

#[cfg(test)]
#[path = "group_dm_tests.rs"]
mod tests;
