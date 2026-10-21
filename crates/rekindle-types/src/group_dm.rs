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
    /// The member's identity key, 64 lowercase hex.
    pub identity: String,
    /// The member's profile record, where its prekey record is found
    /// (plan E2.4's B2 pick) by the first sender of a pair.
    pub profile_record_key: String,
}
