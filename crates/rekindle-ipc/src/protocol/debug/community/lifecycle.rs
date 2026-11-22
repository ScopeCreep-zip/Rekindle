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
                co_creators,
            } => f
                .debug_struct("CommunityCreate")
                .field("name", name)
                .field("description", description)
                .field("approval_required", approval_required)
                .field("co_creators", co_creators)
                .finish(),
            Self::CreatorRemove { anchor, identity } => f
                .debug_struct("CreatorRemove")
                .field("anchor", anchor)
                .field("identity", identity)
                .finish(),
            Self::CommunityCreateCancel { anchor } => f
                .debug_struct("CommunityCreateCancel")
                .field("anchor", anchor)
                .finish(),
            Self::CreatorInviteAnswer { anchor, accept } => f
                .debug_struct("CreatorInviteAnswer")
                .field("anchor", anchor)
                .field("accept", accept)
                .finish(),
            Self::CommunityJoin { invite } => f
                .debug_struct("CommunityJoin")
                .field("invite", invite)
                .finish(),
            Self::CommunityLeave { anchor } => f
                .debug_struct("CommunityLeave")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityForget { anchor } => f
                .debug_struct("CommunityForget")
                .field("anchor", anchor)
                .finish(),
            Self::CoCreationSnapshot => write!(f, "CoCreationSnapshot"),
            Self::CommunityList => write!(f, "CommunityList"),
            Self::CommunityViews => write!(f, "CommunityViews"),
            Self::CommunityMembers { anchor } => f
                .debug_struct("CommunityMembers")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityUpdateInfo {
                anchor,
                name,
                description,
                icon_hash,
                banner_hash,
            } => f
                .debug_struct("CommunityUpdateInfo")
                .field("anchor", anchor)
                .field("name", &name.as_ref().map(String::len))
                .field("description", &description.as_ref().map(String::len))
                .field("icon_hash", icon_hash)
                .field("banner_hash", banner_hash)
                .finish(),
            Self::CommunityInfo { anchor } => f
                .debug_struct("CommunityInfo")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityApprove {
                anchor,
                member_pseudonym,
            } => f
                .debug_struct("CommunityApprove")
                .field("anchor", anchor)
                .field("member_pseudonym", member_pseudonym)
                .finish(),
            Self::CommunityReject {
                anchor,
                member_pseudonym,
                reason,
            } => f
                .debug_struct("CommunityReject")
                .field("anchor", anchor)
                .field("member_pseudonym", member_pseudonym)
                .field("reason", reason)
                .finish(),
            Self::CommunityPendingMembers { anchor } => f
                .debug_struct("CommunityPendingMembers")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityOfferTransfer {
                anchor,
                transfer_to,
            } => f
                .debug_struct("CommunityOfferTransfer")
                .field("anchor", anchor)
                .field("transfer_to", transfer_to)
                .finish(),
            Self::CommunityWithdrawTransfer { anchor } => f
                .debug_struct("CommunityWithdrawTransfer")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityAcceptTransfer { anchor } => f
                .debug_struct("CommunityAcceptTransfer")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityAnswerSuccessorLink { anchor, request } => f
                .debug_struct("CommunityAnswerSuccessorLink")
                .field("anchor", anchor)
                .field("request", request)
                .finish(),
            Self::CommunityPublishSuccessor { anchor, continuing } => f
                .debug_struct("CommunityPublishSuccessor")
                .field("anchor", anchor)
                .field("continuing", &continuing.as_ref().map(Vec::len))
                .finish(),
            Self::CommunityFollowSuccessor { anchor, new_anchor } => f
                .debug_struct("CommunityFollowSuccessor")
                .field("anchor", anchor)
                .field("new_anchor", new_anchor)
                .finish(),
            Self::CommunitySuccessors { anchor } => f
                .debug_struct("CommunitySuccessors")
                .field("anchor", anchor)
                .finish(),
            Self::CommunityDeviceRetire { anchor, device_key } => f
                .debug_struct("CommunityDeviceRetire")
                .field("anchor", anchor)
                .field("device_key", device_key)
                .finish(),
            Self::CommunityDeviceReadmit {
                anchor,
                member,
                device_key,
            } => f
                .debug_struct("CommunityDeviceReadmit")
                .field("anchor", anchor)
                .field("member", member)
                .field("device_key", device_key)
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
