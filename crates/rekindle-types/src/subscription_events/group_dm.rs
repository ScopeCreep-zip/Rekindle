//! Group-DM events (plan E2.6): invites, members joining and leaving, key
//! rotations and the delivery state of our group sends. A peer is named
//! by our nickname for them or their fingerprint, never by a name their
//! payload carries; the group's own name is the creator's.

use serde::{Deserialize, Serialize};

use crate::message::SendState;

/// A member as we show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmMember {
    pub public_key: String,
    /// Our nickname for them, or their fingerprint.
    pub display_name: String,
}

/// Group-DM lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GroupDmEvent {
    /// A creator invited us. Triggered by: a verified `GroupDmInvite`.
    Invited {
        group_id: String,
        /// The creator's name for the group.
        name: String,
        inviter: GroupDmMember,
        /// The signed roster, the inviter and us included.
        members: Vec<GroupDmMember>,
    },
    /// A member (we, or another) joined. Triggered by: our accept, or a
    /// member's `GroupDmAccept`.
    Joined {
        group_id: String,
        member: GroupDmMember,
    },
    /// A member (we, or another) left, or we blocked them. Triggered by:
    /// our leave, a member's `GroupDmLeave`, our block.
    Left {
        group_id: String,
        member: GroupDmMember,
    },
    /// A member's sending key moved to `epoch` (ours after a leave or
    /// block, or a member's resend).
    KeyRotated {
        group_id: String,
        member: GroupDmMember,
        epoch: u32,
    },
    /// Our group send's state: `queued`, `sent` once its chain entry lands,
    /// or `failed`; never `delivered` or `read` (a group has no receipts).
    SendStatus {
        group_id: String,
        client_id: String,
        state: SendState,
    },
}

impl GroupDmEvent {
    /// The group this event belongs to.
    #[must_use]
    pub fn group_id(&self) -> &str {
        match self {
            Self::Invited { group_id, .. }
            | Self::Joined { group_id, .. }
            | Self::Left { group_id, .. }
            | Self::KeyRotated { group_id, .. }
            | Self::SendStatus { group_id, .. } => group_id,
        }
    }
}
