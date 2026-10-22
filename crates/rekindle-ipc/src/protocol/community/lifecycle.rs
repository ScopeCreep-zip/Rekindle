//! The communities: create, join, leave, the lists, the waiting room, ownership and invites (plan E3.1), carried as `CommunityRequest::Lifecycle`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A communities request.
#[derive(Clone, Serialize, Deserialize)]
pub enum LifecycleRequest {
    /// Create a new community.
    CommunityCreate {
        name: String,
        description: String,
        /// Genesis admission mode. `false` (the default) is
        /// `AdmissionMode::Open`; `true` requires an approval before
        /// honest peers count a joiner as a member.
        ///
        /// Chosen at creation and only at creation — the merge honours
        /// `AdmissionPolicy` solely as the genesis entry, so this cannot
        /// be changed later by anyone holding `MANAGE_COMMUNITY`.
        #[serde(default)]
        approval_required: bool,
    },

    /// Join a community via governance key or invite code.
    CommunityJoin { invite: String },

    /// Leave a community.
    CommunityLeave { governance_key: String },

    /// List all joined communities.
    CommunityList,

    /// Every joined community as its window renders it (channels,
    /// categories, roles, our standing).
    CommunityViews,

    /// A community's member list.
    CommunityMembers { governance_key: String },

    /// Get detailed community info (channels, roles, members).
    CommunityInfo { governance_key: String },

    /// Edit a community's name, description, icon or banner; `None` keeps
    /// the merged value.
    CommunityUpdateInfo {
        governance_key: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
    },

    /// Approve a pending member from the waiting room.
    CommunityApprove {
        governance_key: String,
        member_pseudonym: String,
    },

    /// Reject a pending member from the waiting room.
    CommunityReject {
        governance_key: String,
        member_pseudonym: String,
        reason: String,
    },

    /// List pending join requests for a community.
    CommunityPendingMembers { governance_key: String },

    /// Transfer community ownership to a new owner.
    CommunityTransferOwnership {
        governance_key: String,
        new_owner_pseudonym: String,
    },

    /// Create a community invite.
    InviteCreate {
        community: String,
        max_uses: u32,
        expires_seconds: Option<u64>,
    },

    /// List active invites for a community.
    InviteList { community: String },

    /// Revoke an invite by code.
    InviteRevoke {
        community: String,
        invite_code: String,
    },
}

impl LifecycleRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::CommunityCreate { .. } => ("CommunityCreate", Write, Authenticated),
            Self::CommunityJoin { .. } => ("CommunityJoin", Write, Authenticated),
            Self::CommunityLeave { .. } => ("CommunityLeave", Write, Authenticated),
            Self::CommunityList => ("CommunityList", Query, Authenticated),
            Self::CommunityViews => ("CommunityViews", Query, Authenticated),
            Self::CommunityMembers { .. } => ("CommunityMembers", Query, Authenticated),
            Self::CommunityInfo { .. } => ("CommunityInfo", Query, Authenticated),
            Self::CommunityUpdateInfo { .. } => ("CommunityUpdateInfo", Write, Authenticated),
            Self::CommunityApprove { .. } => ("CommunityApprove", Write, Authenticated),
            Self::CommunityReject { .. } => ("CommunityReject", Write, Authenticated),
            Self::CommunityPendingMembers { .. } => {
                ("CommunityPendingMembers", Query, Authenticated)
            }
            Self::CommunityTransferOwnership { .. } => {
                ("CommunityTransferOwnership", Write, Authenticated)
            }
            Self::InviteCreate { .. } => ("InviteCreate", Write, Authenticated),
            Self::InviteList { .. } => ("InviteList", Query, Authenticated),
            Self::InviteRevoke { .. } => ("InviteRevoke", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, or
    /// `None` when no agent may send it.
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        use AgentCapability as C;
        match self {
            Self::CommunityList
            | Self::CommunityViews
            | Self::CommunityMembers { .. }
            | Self::CommunityInfo { .. }
            | Self::CommunityPendingMembers { .. }
            | Self::InviteList { .. } => Some(C::CommunitiesRead),
            Self::CommunityCreate { .. }
            | Self::CommunityJoin { .. }
            | Self::CommunityLeave { .. }
            | Self::CommunityUpdateInfo { .. }
            | Self::InviteCreate { .. }
            | Self::InviteRevoke { .. } => Some(C::CommunitiesWrite),
            Self::CommunityApprove { .. } | Self::CommunityReject { .. } => {
                Some(C::ModerationWrite)
            }
            // Channel keys are key material; ownership is the owner's own.
            Self::CommunityTransferOwnership { .. } => None,
        }
    }
}
