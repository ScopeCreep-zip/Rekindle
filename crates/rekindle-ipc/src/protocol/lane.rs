//! Request classification: each `IpcRequest`'s stable name (for the audit
//! log) and the dispatch lane it runs in.

use super::IpcRequest;

/// How a request may overlap the requests in flight with it.
///
/// The daemon dispatches every request as its own task. `Write` requests
/// share a lane with each other; an `Exclusive` request — one that loads,
/// replaces or removes the identity or its signing key — waits for every
/// in-flight write and holds the lane alone, so no write that cloned the
/// signing key can complete after a lock. `Query` requests take no lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Query,
    Write,
    Exclusive,
}

impl IpcRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    /// The dispatch lane this request runs in.
    #[must_use]
    pub fn lane(&self) -> Lane {
        self.class().1
    }

    fn class(&self) -> (&'static str, Lane) {
        use Lane::{Exclusive, Query, Write};
        match self {
            Self::Unlock { .. } => ("Unlock", Exclusive),
            Self::Lock => ("Lock", Exclusive),
            Self::Status => ("Status", Query),
            Self::Shutdown => ("Shutdown", Exclusive),

            Self::IdentityCreate { .. } => ("IdentityCreate", Exclusive),
            Self::IdentityShow => ("IdentityShow", Query),
            Self::IdentityExport => ("IdentityExport", Query),
            Self::IdentityRotate => ("IdentityRotate", Exclusive),
            Self::IdentityDestroy { .. } => ("IdentityDestroy", Exclusive),
            Self::IdentityWipe { .. } => ("IdentityWipe", Exclusive),

            Self::FriendAdd { .. } => ("FriendAdd", Write),
            Self::FriendAccept { .. } => ("FriendAccept", Write),
            Self::FriendReject { .. } => ("FriendReject", Write),
            Self::FriendRemove { .. } => ("FriendRemove", Write),
            Self::FriendList => ("FriendList", Query),
            Self::FriendRequests => ("FriendRequests", Query),

            Self::CommunityCreate { .. } => ("CommunityCreate", Write),
            Self::CommunityJoin { .. } => ("CommunityJoin", Write),
            Self::CommunityLeave { .. } => ("CommunityLeave", Write),
            Self::CommunityList => ("CommunityList", Query),
            Self::CommunityInfo { .. } => ("CommunityInfo", Query),
            Self::CommunityApprove { .. } => ("CommunityApprove", Write),
            Self::CommunityReject { .. } => ("CommunityReject", Write),
            Self::CommunityPendingMembers { .. } => ("CommunityPendingMembers", Query),
            Self::CommunityTransferOwnership { .. } => ("CommunityTransferOwnership", Write),

            Self::ChannelList { .. } => ("ChannelList", Query),
            Self::ChannelCreate { .. } => ("ChannelCreate", Write),
            Self::ChannelDelete { .. } => ("ChannelDelete", Write),
            Self::ChannelUpdate { .. } => ("ChannelUpdate", Write),
            Self::ChannelSend { .. } => ("ChannelSend", Write),
            Self::ChannelHistory { .. } => ("ChannelHistory", Query),

            Self::DmSend { .. } => ("DmSend", Write),
            Self::DmTyping { .. } => ("DmTyping", Write),
            Self::DmInbox { .. } => ("DmInbox", Query),

            Self::Subscribe { .. } => ("Subscribe", Query),
            Self::Unsubscribe { .. } => ("Unsubscribe", Query),

            Self::MekList { .. } => ("MekList", Query),
            Self::MekRotate { .. } => ("MekRotate", Write),
            Self::MekRequest { .. } => ("MekRequest", Write),
            Self::PrekeyReplenish => ("PrekeyReplenish", Write),

            Self::PresenceSet { .. } => ("PresenceSet", Write),
            Self::GamePresenceSet { .. } => ("GamePresenceSet", Write),
            Self::GamePresenceClear => ("GamePresenceClear", Write),

            Self::RoleList { .. } => ("RoleList", Query),
            Self::RoleCreate { .. } => ("RoleCreate", Write),
            Self::RoleUpdate { .. } => ("RoleUpdate", Write),
            Self::RoleDelete { .. } => ("RoleDelete", Write),
            Self::RoleAssign { .. } => ("RoleAssign", Write),
            Self::RoleUnassign { .. } => ("RoleUnassign", Write),

            Self::Kick { .. } => ("Kick", Write),
            Self::Ban { .. } => ("Ban", Write),
            Self::Unban { .. } => ("Unban", Write),
            Self::Timeout { .. } => ("Timeout", Write),
            Self::BanList { .. } => ("BanList", Query),

            Self::InviteCreate { .. } => ("InviteCreate", Write),
            Self::InviteList { .. } => ("InviteList", Query),
            Self::InviteRevoke { .. } => ("InviteRevoke", Write),

            Self::VoiceJoin { .. } => ("VoiceJoin", Write),
            Self::VoiceLeave => ("VoiceLeave", Write),

            Self::NetworkStatus => ("NetworkStatus", Query),
            Self::NetworkPeers => ("NetworkPeers", Query),

            Self::AgentRegister { .. } => ("AgentRegister", Write),
            Self::AgentRevoke { .. } => ("AgentRevoke", Write),
            Self::PolicyReload => ("PolicyReload", Write),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_key_changes_are_exclusive() {
        for request in [
            IpcRequest::Lock,
            IpcRequest::Unlock {
                passphrase: String::new(),
            },
            IpcRequest::Shutdown,
            IpcRequest::IdentityRotate,
            IpcRequest::IdentityWipe {
                confirmation: String::new(),
            },
        ] {
            assert_eq!(request.lane(), Lane::Exclusive, "{}", request.name());
        }
        assert_eq!(IpcRequest::Status.lane(), Lane::Query);
        assert_eq!(
            IpcRequest::DmTyping {
                peer_key: String::new(),
                typing: true
            }
            .lane(),
            Lane::Write
        );
    }

    #[test]
    fn name_omits_fields() {
        let request = IpcRequest::Unlock {
            passphrase: "secret".into(),
        };
        assert_eq!(request.name(), "Unlock");
    }
}
