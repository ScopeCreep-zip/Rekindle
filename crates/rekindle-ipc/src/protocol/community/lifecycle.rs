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
        /// Friends (identity keys) named co-creators (plan E3.2 S7s.3):
        /// each is invited and the genesis lists those that accept.
        #[serde(default)]
        co_creators: Vec<String>,
    },

    /// Stop waiting on `identity`, a co-creator of our pending creation
    /// of `anchor` that has not answered; it is told (plan E3.2 S7s.3).
    CreatorRemove { anchor: String, identity: String },

    /// Cancel our pending creation of `anchor`: the records are released
    /// and every co-creator is told (plan E3.2 S7s.3).
    CommunityCreateCancel { anchor: String },

    /// Answer a held co-creator invite to `anchor` (plan E3.2 S7s.3).
    CreatorInviteAnswer { anchor: String, accept: bool },

    /// Join a community via anchor key or invite code.
    CommunityJoin { invite: String },

    /// Leave a community.
    CommunityLeave { anchor: String },

    /// Delete a community we were removed from: its kept history, keys
    /// and every local row (plan E3.2 S9 R3-5).
    CommunityForget { anchor: String },

    /// The held co-creator invites and our pending creations' co-creator
    /// states (plan E3.2 S7s.3g), read once a window's event stream is
    /// live.
    CoCreationSnapshot,

    /// List all joined communities.
    CommunityList,

    /// Every joined community as its window renders it (channels,
    /// categories, roles, our standing).
    CommunityViews,

    /// A community's member list.
    CommunityMembers { anchor: String },

    /// Get detailed community info (channels, roles, members).
    CommunityInfo { anchor: String },

    /// Edit a community's name, description, icon or banner; `None` keeps
    /// the merged value.
    CommunityUpdateInfo {
        anchor: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
    },

    /// Approve a pending member from the waiting room.
    CommunityApprove {
        anchor: String,
        member_pseudonym: String,
    },

    /// Reject a pending member from the waiting room.
    CommunityReject {
        anchor: String,
        member_pseudonym: String,
        reason: String,
    },

    /// List pending join requests for a community.
    CommunityPendingMembers { anchor: String },

    /// Offer `anchor` to the member `transfer_to` (pseudonym hex): a
    /// transfer, completed only by that member's acceptance (plan E3.2
    /// S7s.4, lead decisions (24)–(26)). Creator-only; our latest offer or
    /// withdrawal supersedes our earlier ones.
    CommunityOfferTransfer { anchor: String, transfer_to: String },

    /// Withdraw our standing transfer offer of `anchor` (decision (26)).
    CommunityWithdrawTransfer { anchor: String },

    /// Accept the standing transfer offer of `anchor` to us: collect a
    /// creator link from each of the offer's other creators through
    /// `co_creators` (friends' identity keys, one per other creator;
    /// decision (16)'s round, which removes no one, decision (25)), then
    /// create the successor and publish the acceptance.
    CommunityAcceptTransfer {
        anchor: String,
        co_creators: Vec<String>,
    },

    /// Re-create `anchor` under a new anchor whose cert names it as
    /// `predecessor`, with us as its sole creator, and name the new anchor
    /// here as its successor (ADR 0011 "Successor community", plan E3.2
    /// S7s.4). Creator-only. `continuing` names the members (pseudonym
    /// hex) who get a sealed invite to it; absent, every current member
    /// does (lead decision (34)).
    CommunityPublishSuccessor {
        anchor: String,
        continuing: Option<Vec<String>>,
    },

    /// Follow `new_anchor`, a successor of `anchor`, by joining it with our
    /// invite sealed in its `invitesRecord` (lead decision (34)): both links
    /// are re-checked first and `anchor` is kept (plan E3.2 S7s.4).
    CommunityFollowSuccessor { anchor: String, new_anchor: String },

    /// The successors of `anchor` offered this session (lead decision
    /// (33)'s snapshot of `CommunitySuccessorOffered`).
    CommunitySuccessors { anchor: String },

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
            Self::CreatorRemove { .. } => ("CreatorRemove", Write, Authenticated),
            Self::CommunityCreateCancel { .. } => ("CommunityCreateCancel", Write, Authenticated),
            Self::CreatorInviteAnswer { .. } => ("CreatorInviteAnswer", Write, Authenticated),
            Self::CommunityJoin { .. } => ("CommunityJoin", Write, Authenticated),
            Self::CommunityLeave { .. } => ("CommunityLeave", Write, Authenticated),
            Self::CommunityForget { .. } => ("CommunityForget", Write, Authenticated),
            Self::CoCreationSnapshot => ("CoCreationSnapshot", Query, Authenticated),
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
            Self::CommunityOfferTransfer { .. } => ("CommunityOfferTransfer", Write, Authenticated),
            Self::CommunityWithdrawTransfer { .. } => {
                ("CommunityWithdrawTransfer", Write, Authenticated)
            }
            Self::CommunityAcceptTransfer { .. } => {
                ("CommunityAcceptTransfer", Write, Authenticated)
            }
            Self::CommunityPublishSuccessor { .. } => {
                ("CommunityPublishSuccessor", Write, Authenticated)
            }
            Self::CommunityFollowSuccessor { .. } => {
                ("CommunityFollowSuccessor", Write, Authenticated)
            }
            Self::CommunitySuccessors { .. } => ("CommunitySuccessors", Query, Authenticated),
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
            | Self::CoCreationSnapshot
            | Self::CommunityViews
            | Self::CommunityMembers { .. }
            | Self::CommunityInfo { .. }
            | Self::CommunityPendingMembers { .. }
            | Self::CommunitySuccessors { .. }
            | Self::InviteList { .. } => Some(C::CommunitiesRead),
            Self::CommunityCreate { .. }
            | Self::CreatorRemove { .. }
            | Self::CommunityCreateCancel { .. }
            | Self::CreatorInviteAnswer { .. }
            | Self::CommunityJoin { .. }
            | Self::CommunityFollowSuccessor { .. }
            | Self::CommunityAcceptTransfer { .. }
            | Self::CommunityLeave { .. }
            | Self::CommunityForget { .. }
            | Self::CommunityUpdateInfo { .. }
            | Self::InviteCreate { .. }
            | Self::InviteRevoke { .. } => Some(C::CommunitiesWrite),
            Self::CommunityApprove { .. } | Self::CommunityReject { .. } => {
                Some(C::ModerationWrite)
            }
            // Channel keys are key material; ownership is the owner's own.
            // A successor moves creator authority: the creator's own act.
            Self::CommunityOfferTransfer { .. }
            | Self::CommunityWithdrawTransfer { .. }
            | Self::CommunityPublishSuccessor { .. } => None,
        }
    }
}
