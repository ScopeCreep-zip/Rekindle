//! Agent capabilities (plan D1): the closed vocabulary an `Agent`-level
//! connection is scoped by, in Slack's `resource:action` form
//! (docs.slack.dev/reference/scopes: `channels:read`, `channels:history`,
//! `chat:write`, `im:history`, …). An agent may send a request above `Open`
//! only when its registration holds the request's capability; a request
//! with none (key material, identity moves, shutdown, administration) is
//! never delegated to an agent.

use super::IpcRequest;

/// One agent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentCapability {
    /// The identity's public profile (`IdentityShow`).
    IdentityRead,
    /// Network state (`NetworkStatus`, `NetworkPeers`).
    NetworkRead,
    /// The event stream (`Subscribe`, `Unsubscribe`).
    EventsRead,
    FriendsRead,
    FriendsWrite,
    /// Communities, their channels, roles, bans and invites, as listed.
    CommunitiesRead,
    /// Create, join and leave communities; create and change channels,
    /// roles and invites.
    CommunitiesWrite,
    /// Admit, refuse, kick, ban, time out, assign roles.
    ModerationWrite,
    /// Channel message history.
    ChannelsHistory,
    /// Send channel messages.
    ChatWrite,
    /// Direct message history.
    ImHistory,
    /// Send direct messages and typing.
    ImWrite,
    PresenceWrite,
    VoiceWrite,
}

impl AgentCapability {
    /// Every capability.
    pub const ALL: &[Self] = &[
        Self::IdentityRead,
        Self::NetworkRead,
        Self::EventsRead,
        Self::FriendsRead,
        Self::FriendsWrite,
        Self::CommunitiesRead,
        Self::CommunitiesWrite,
        Self::ModerationWrite,
        Self::ChannelsHistory,
        Self::ChatWrite,
        Self::ImHistory,
        Self::ImWrite,
        Self::PresenceWrite,
        Self::VoiceWrite,
    ];

    /// The capability's wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IdentityRead => "identity:read",
            Self::NetworkRead => "network:read",
            Self::EventsRead => "events:read",
            Self::FriendsRead => "friends:read",
            Self::FriendsWrite => "friends:write",
            Self::CommunitiesRead => "communities:read",
            Self::CommunitiesWrite => "communities:write",
            Self::ModerationWrite => "moderation:write",
            Self::ChannelsHistory => "channels:history",
            Self::ChatWrite => "chat:write",
            Self::ImHistory => "im:history",
            Self::ImWrite => "im:write",
            Self::PresenceWrite => "presence:write",
            Self::VoiceWrite => "voice:write",
        }
    }

    /// The capability a wire name names, if any.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.as_str() == name)
    }
}

impl IpcRequest {
    /// The capability an `Agent`-level sender needs for this request, or
    /// `None` when no agent may send it. Requests that need only `Open`
    /// clearance are not capability-checked.
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        use AgentCapability as C;
        match self {
            Self::IdentityShow | Self::PresenceGet | Self::AvatarGet { .. } => {
                Some(C::IdentityRead)
            }
            Self::NetworkStatus | Self::NetworkPeers => Some(C::NetworkRead),
            Self::Subscribe { .. } | Self::Unsubscribe { .. } => Some(C::EventsRead),
            Self::FriendList
            | Self::FriendRequests
            | Self::FriendInviteList
            | Self::FriendBlockedList => Some(C::FriendsRead),
            Self::FriendAdd { .. }
            | Self::FriendAccept { .. }
            | Self::FriendReject { .. }
            | Self::FriendRemove { .. }
            | Self::FriendCancelRequest { .. }
            | Self::FriendAddFromInvite { .. }
            | Self::FriendInviteCreate
            | Self::FriendInviteCancel { .. }
            | Self::FriendBlock { .. }
            | Self::FriendUnblock { .. }
            | Self::FriendSetNickname { .. }
            | Self::FriendGroupCreate { .. }
            | Self::FriendGroupRename { .. }
            | Self::FriendGroupMove { .. }
            | Self::FriendGroupDelete { .. }
            | Self::SessionResetStart { .. }
            | Self::SessionResetAccept { .. }
            | Self::SessionResetDecline { .. } => Some(C::FriendsWrite),
            Self::CommunityList
            | Self::CommunityInfo { .. }
            | Self::CommunityPendingMembers { .. }
            | Self::ChannelList { .. }
            | Self::RoleList { .. }
            | Self::BanList { .. }
            | Self::InviteList { .. } => Some(C::CommunitiesRead),
            Self::CommunityCreate { .. }
            | Self::CommunityJoin { .. }
            | Self::CommunityLeave { .. }
            | Self::ChannelCreate { .. }
            | Self::ChannelDelete { .. }
            | Self::ChannelUpdate { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::InviteCreate { .. }
            | Self::InviteRevoke { .. } => Some(C::CommunitiesWrite),
            Self::CommunityApprove { .. }
            | Self::CommunityReject { .. }
            | Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. } => Some(C::ModerationWrite),
            Self::ChannelHistory { .. } => Some(C::ChannelsHistory),
            Self::ChannelSend { .. } => Some(C::ChatWrite),
            Self::DmInbox { .. } | Self::DmHistory { .. } => Some(C::ImHistory),
            Self::DmSend { .. } | Self::DmTyping { .. } | Self::DmMarkRead { .. } => {
                Some(C::ImWrite)
            }
            Self::PresenceSet { .. }
            | Self::PresenceSetMessage { .. }
            | Self::GamePresenceSet { .. }
            | Self::GamePresenceClear => Some(C::PresenceWrite),
            Self::VoiceJoin { .. } | Self::VoiceLeave => Some(C::VoiceWrite),
            Self::Unlock { .. }
            | Self::Lock
            | Self::Logout
            | Self::Status
            | Self::Shutdown
            | Self::IdentityCreate { .. }
            | Self::IdentityList
            | Self::IdentityExport { .. }
            | Self::IdentityImport { .. }
            | Self::IdentityRotate { .. }
            | Self::IdentityDelete { .. }
            | Self::IdentityWipe
            // A commit runs the request the gate held, which passed this
            // caller's capability check when it was selected.
            | Self::IntentCommit { .. }
            | Self::IntentCancel { .. }
            // An undo's inverse passes this check on its own at the gate.
            | Self::Undo { .. }
            | Self::MekList { .. }
            | Self::MekRotate { .. }
            | Self::MekRequest { .. }
            | Self::PrekeyReplenish
            // Our name, our picture, the audit log and the system browser
            // are the user's own, never an agent's.
            | Self::ProfileSetNickname { .. }
            | Self::ProfileSetAvatar { .. }
            | Self::AuditVerify
            | Self::AuditExport { .. }
            | Self::LinkOpen { .. }
            | Self::CommunityTransferOwnership { .. }
            | Self::AgentRegister { .. }
            | Self::AgentRevoke { .. }
            | Self::PolicyReload => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_round_trips_by_name() {
        for capability in AgentCapability::ALL {
            assert_eq!(
                AgentCapability::parse(capability.as_str()),
                Some(*capability)
            );
        }
        assert_eq!(AgentCapability::parse("chat"), None);
    }

    #[test]
    fn key_material_is_never_delegated() {
        assert_eq!(IpcRequest::PrekeyReplenish.agent_capability(), None);
        assert_eq!(
            IpcRequest::IdentityExport {
                passphrase: String::new()
            }
            .agent_capability(),
            None
        );
        assert_eq!(
            IpcRequest::DmInbox { limit: 10 }.agent_capability(),
            Some(AgentCapability::ImHistory)
        );
    }
}
