//! Request classification: each `IpcRequest`'s stable name (for the audit
//! log), the dispatch lane it runs in, and the clearance it requires.

use super::IpcRequest;
use crate::message::SecurityLevel;

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

    /// The clearance the sending connection needs (plan D1), checked by the
    /// daemon against the level the bus server stamped. `Open`: the
    /// locked-state requests, each proved by a passphrase or a typed phrase
    /// (and `Status`). `Admin`: agents, policy, and moving the identity's
    /// key (rotate, export). `Authenticated`: everything else.
    #[must_use]
    pub fn required_clearance(&self) -> SecurityLevel {
        self.class().2
    }

    fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Exclusive, Query, Write};
        use SecurityLevel::{Admin, Authenticated, Open};
        match self {
            Self::Unlock { .. } => ("Unlock", Exclusive, Open),
            Self::Lock => ("Lock", Exclusive, Open),
            Self::Logout => ("Logout", Exclusive, Open),
            Self::Status => ("Status", Query, Open),
            Self::Shutdown => ("Shutdown", Exclusive, Authenticated),

            Self::IdentityCreate { .. } => ("IdentityCreate", Exclusive, Open),
            Self::IdentityList => ("IdentityList", Query, Open),
            Self::IdentityShow => ("IdentityShow", Query, Authenticated),
            Self::IdentityExport { .. } => ("IdentityExport", Query, Admin),
            Self::IdentityImport { .. } => ("IdentityImport", Exclusive, Open),
            Self::IdentityRotate { .. } => ("IdentityRotate", Exclusive, Admin),
            Self::IdentityDelete { .. } => ("IdentityDelete", Exclusive, Open),
            Self::IdentityWipe => ("IdentityWipe", Exclusive, Open),
            // The gate checks who may commit (plan D3.3); the held request
            // runs in its own lane.
            Self::IntentCommit { .. } => ("IntentCommit", Write, Open),
            Self::IntentCancel { .. } => ("IntentCancel", Write, Open),

            Self::FriendAdd { .. } => ("FriendAdd", Write, Authenticated),
            Self::FriendAccept { .. } => ("FriendAccept", Write, Authenticated),
            Self::FriendReject { .. } => ("FriendReject", Write, Authenticated),
            Self::FriendRemove { .. } => ("FriendRemove", Write, Authenticated),
            Self::FriendList => ("FriendList", Query, Authenticated),
            Self::FriendRequests => ("FriendRequests", Query, Authenticated),
            Self::FriendCancelRequest { .. } => ("FriendCancelRequest", Write, Authenticated),
            Self::FriendAddFromInvite { .. } => ("FriendAddFromInvite", Write, Authenticated),
            Self::FriendInviteCreate => ("FriendInviteCreate", Write, Authenticated),
            Self::FriendInviteList => ("FriendInviteList", Query, Authenticated),
            Self::FriendInviteCancel { .. } => ("FriendInviteCancel", Write, Authenticated),
            Self::FriendBlock { .. } => ("FriendBlock", Write, Authenticated),
            Self::FriendUnblock { .. } => ("FriendUnblock", Write, Authenticated),
            Self::FriendBlockedList => ("FriendBlockedList", Query, Authenticated),
            Self::FriendSetNickname { .. } => ("FriendSetNickname", Write, Authenticated),
            Self::FriendGroupCreate { .. } => ("FriendGroupCreate", Write, Authenticated),
            Self::FriendGroupRename { .. } => ("FriendGroupRename", Write, Authenticated),
            Self::FriendGroupMove { .. } => ("FriendGroupMove", Write, Authenticated),
            Self::FriendGroupDelete { .. } => ("FriendGroupDelete", Write, Authenticated),
            Self::SessionResetStart { .. } => ("SessionResetStart", Write, Authenticated),
            Self::SessionResetAccept { .. } => ("SessionResetAccept", Write, Authenticated),
            Self::SessionResetDecline { .. } => ("SessionResetDecline", Write, Authenticated),

            Self::CommunityCreate { .. } => ("CommunityCreate", Write, Authenticated),
            Self::CommunityJoin { .. } => ("CommunityJoin", Write, Authenticated),
            Self::CommunityLeave { .. } => ("CommunityLeave", Write, Authenticated),
            Self::CommunityList => ("CommunityList", Query, Authenticated),
            Self::CommunityInfo { .. } => ("CommunityInfo", Query, Authenticated),
            Self::CommunityApprove { .. } => ("CommunityApprove", Write, Authenticated),
            Self::CommunityReject { .. } => ("CommunityReject", Write, Authenticated),
            Self::CommunityPendingMembers { .. } => {
                ("CommunityPendingMembers", Query, Authenticated)
            }
            Self::CommunityTransferOwnership { .. } => {
                ("CommunityTransferOwnership", Write, Authenticated)
            }

            Self::ChannelList { .. } => ("ChannelList", Query, Authenticated),
            Self::ChannelCreate { .. } => ("ChannelCreate", Write, Authenticated),
            Self::ChannelDelete { .. } => ("ChannelDelete", Write, Authenticated),
            Self::ChannelUpdate { .. } => ("ChannelUpdate", Write, Authenticated),
            Self::ChannelSend { .. } => ("ChannelSend", Write, Authenticated),
            Self::ChannelHistory { .. } => ("ChannelHistory", Query, Authenticated),

            Self::DmSend { .. } => ("DmSend", Write, Authenticated),
            Self::DmTyping { .. } => ("DmTyping", Write, Authenticated),
            Self::DmInbox { .. } => ("DmInbox", Query, Authenticated),
            Self::DmHistory { .. } => ("DmHistory", Query, Authenticated),
            Self::DmMarkRead { .. } => ("DmMarkRead", Write, Authenticated),

            Self::Undo { .. } => ("Undo", Write, Authenticated),
            Self::Subscribe { .. } => ("Subscribe", Query, Authenticated),
            Self::Unsubscribe { .. } => ("Unsubscribe", Query, Authenticated),

            Self::MekList { .. } => ("MekList", Query, Authenticated),
            Self::MekRotate { .. } => ("MekRotate", Write, Authenticated),
            Self::MekRequest { .. } => ("MekRequest", Write, Authenticated),
            Self::PrekeyReplenish => ("PrekeyReplenish", Write, Authenticated),

            Self::PresenceSet { .. } => ("PresenceSet", Write, Authenticated),
            Self::PresenceSetMessage { .. } => ("PresenceSetMessage", Write, Authenticated),
            Self::PresenceGet => ("PresenceGet", Query, Authenticated),
            Self::ProfileSetNickname { .. } => ("ProfileSetNickname", Write, Authenticated),
            Self::ProfileSetAvatar { .. } => ("ProfileSetAvatar", Write, Authenticated),
            Self::AvatarGet { .. } => ("AvatarGet", Query, Authenticated),
            Self::GamePresenceSet { .. } => ("GamePresenceSet", Write, Authenticated),
            Self::GamePresenceClear => ("GamePresenceClear", Write, Authenticated),

            Self::AuditVerify => ("AuditVerify", Query, Authenticated),
            Self::AuditExport { .. } => ("AuditExport", Query, Authenticated),
            Self::LinkOpen { .. } => ("LinkOpen", Query, Authenticated),

            Self::RoleList { .. } => ("RoleList", Query, Authenticated),
            Self::RoleCreate { .. } => ("RoleCreate", Write, Authenticated),
            Self::RoleUpdate { .. } => ("RoleUpdate", Write, Authenticated),
            Self::RoleDelete { .. } => ("RoleDelete", Write, Authenticated),
            Self::RoleAssign { .. } => ("RoleAssign", Write, Authenticated),
            Self::RoleUnassign { .. } => ("RoleUnassign", Write, Authenticated),

            Self::Kick { .. } => ("Kick", Write, Authenticated),
            Self::Ban { .. } => ("Ban", Write, Authenticated),
            Self::Unban { .. } => ("Unban", Write, Authenticated),
            Self::Timeout { .. } => ("Timeout", Write, Authenticated),
            Self::BanList { .. } => ("BanList", Query, Authenticated),

            Self::InviteCreate { .. } => ("InviteCreate", Write, Authenticated),
            Self::InviteList { .. } => ("InviteList", Query, Authenticated),
            Self::InviteRevoke { .. } => ("InviteRevoke", Write, Authenticated),

            Self::VoiceJoin { .. } => ("VoiceJoin", Write, Authenticated),
            Self::VoiceLeave => ("VoiceLeave", Write, Authenticated),

            Self::NetworkStatus => ("NetworkStatus", Query, Authenticated),
            Self::NetworkPeers => ("NetworkPeers", Query, Authenticated),

            Self::AgentRegister { .. } => ("AgentRegister", Write, Admin),
            Self::AgentRevoke { .. } => ("AgentRevoke", Write, Admin),
            Self::PolicyReload => ("PolicyReload", Write, Admin),
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
                identity: String::new(),
                passphrase: String::new(),
            },
            IpcRequest::Logout,
            IpcRequest::Shutdown,
            IpcRequest::IdentityRotate {
                passphrase: String::new(),
            },
            IpcRequest::IdentityImport {
                archive: String::new(),
                passphrase: String::new(),
            },
            IpcRequest::IdentityWipe,
        ] {
            assert_eq!(request.lane(), Lane::Exclusive, "{}", request.name());
        }
        assert_eq!(IpcRequest::Status.lane(), Lane::Query);
        assert_eq!(IpcRequest::IdentityList.lane(), Lane::Query);
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
    fn locked_state_requests_are_open_and_key_moves_are_admin() {
        assert_eq!(IpcRequest::Status.required_clearance(), SecurityLevel::Open);
        assert_eq!(
            IpcRequest::Unlock {
                identity: String::new(),
                passphrase: String::new(),
            }
            .required_clearance(),
            SecurityLevel::Open
        );
        assert_eq!(
            IpcRequest::IdentityExport {
                passphrase: String::new(),
            }
            .required_clearance(),
            SecurityLevel::Admin
        );
        assert_eq!(
            IpcRequest::PolicyReload.required_clearance(),
            SecurityLevel::Admin
        );
        assert_eq!(
            IpcRequest::FriendList.required_clearance(),
            SecurityLevel::Authenticated
        );
    }

    #[test]
    fn name_omits_fields() {
        let request = IpcRequest::Unlock {
            identity: "pk".into(),
            passphrase: "secret".into(),
        };
        assert_eq!(request.name(), "Unlock");
    }
}
