//! Group-DM DTOs (plan E2.6): a group as the buddy list and the chat
//! window show it. Its messages are [`super::dm::DmMessage`] rows.

use serde::{Deserialize, Serialize};

/// One member of a group, named by our nickname for them or their
/// fingerprint, never by a name their payload carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmViewMember {
    /// Their identity key, 64 lowercase hex.
    pub key: String,
    pub label: String,
}

/// A group we are in, or an invite to one we have not answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDmView {
    /// The creator's 16 random bytes, 32 lowercase hex.
    pub group_id: String,
    /// The creator's name for it.
    pub name: String,
    /// Every member on the signed roster, the creator first.
    pub members: Vec<GroupDmViewMember>,
    /// Our sending key's epoch; the creator's for a pending invite.
    pub epoch: u32,
    /// An invite we have not accepted or declined.
    pub pending: bool,
}
