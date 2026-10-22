//! The community lifecycle, membership and invite requests' `Debug`
//! (plan E3.1): free text shows its length only.

use crate::protocol::LifecycleRequest;

impl std::fmt::Debug for LifecycleRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommunityCreate {
                name,
                description,
                approval_required,
            } => f
                .debug_struct("CommunityCreate")
                .field("name", name)
                .field("description", description)
                .field("approval_required", approval_required)
                .finish(),
            Self::CommunityJoin { invite } => f
                .debug_struct("CommunityJoin")
                .field("invite", invite)
                .finish(),
            Self::CommunityLeave { governance_key } => f
                .debug_struct("CommunityLeave")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityList => write!(f, "CommunityList"),
            Self::CommunityViews => write!(f, "CommunityViews"),
            Self::CommunityMembers { governance_key } => f
                .debug_struct("CommunityMembers")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityUpdateInfo {
                governance_key,
                name,
                description,
                icon_hash,
                banner_hash,
            } => f
                .debug_struct("CommunityUpdateInfo")
                .field("governance_key", governance_key)
                .field("name", &name.as_ref().map(String::len))
                .field("description", &description.as_ref().map(String::len))
                .field("icon_hash", icon_hash)
                .field("banner_hash", banner_hash)
                .finish(),
            Self::CommunityInfo { governance_key } => f
                .debug_struct("CommunityInfo")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityApprove {
                governance_key,
                member_pseudonym,
            } => f
                .debug_struct("CommunityApprove")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .finish(),
            Self::CommunityReject {
                governance_key,
                member_pseudonym,
                reason,
            } => f
                .debug_struct("CommunityReject")
                .field("governance_key", governance_key)
                .field("member_pseudonym", member_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::CommunityPendingMembers { governance_key } => f
                .debug_struct("CommunityPendingMembers")
                .field("governance_key", governance_key)
                .finish(),
            Self::CommunityTransferOwnership {
                governance_key,
                new_owner_pseudonym,
            } => f
                .debug_struct("CommunityTransferOwnership")
                .field("governance_key", governance_key)
                .field("new_owner_pseudonym", new_owner_pseudonym)
                .finish(),
            Self::InviteCreate {
                community,
                max_uses,
                expires_seconds,
            } => f
                .debug_struct("InviteCreate")
                .field("community", community)
                .field("max_uses", max_uses)
                .field("expires_seconds", expires_seconds)
                .finish(),
            Self::InviteList { community } => f
                .debug_struct("InviteList")
                .field("community", community)
                .finish(),
            Self::InviteRevoke {
                community,
                invite_code,
            } => f
                .debug_struct("InviteRevoke")
                .field("community", community)
                .field("invite_code", invite_code)
                .finish(),
        }
    }
}
