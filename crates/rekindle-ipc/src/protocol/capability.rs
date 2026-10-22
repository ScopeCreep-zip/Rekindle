//! Agent capabilities (plan D1): the closed vocabulary an `Agent`-level
//! connection is scoped by, in Slack's `resource:action` form
//! (docs.slack.dev/reference/scopes: `channels:read`, `channels:history`,
//! `chat:write`, `im:history`, …). An agent may send a request above `Open`
//! only when its registration holds the request's capability; a request
//! with none (key material, identity moves, shutdown, administration) is
//! never delegated to an agent.

use super::{CommunityRequest, IpcRequest};

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
            | Self::FriendBlockedList
            | Self::FriendCode
            | Self::IdentitySafetyNumber { .. } => Some(C::FriendsRead),
            Self::FriendAddFromCode { .. }
            | Self::FriendAccept { .. }
            | Self::FriendReject { .. }
            | Self::FriendRemove { .. }
            | Self::FriendCancelRequest { .. }
            | Self::FriendRetryHandshake { .. }
            | Self::FriendAddFromInvite { .. }
            | Self::FriendInviteCreate
            | Self::FriendTokenRotate
            | Self::FriendInviteCancel { .. }
            | Self::FriendBlock { .. }
            | Self::FriendUnblock { .. }
            | Self::FriendSetNickname { .. }
            | Self::FriendGroupCreate { .. }
            | Self::FriendGroupRename { .. }
            | Self::FriendGroupMove { .. }
            | Self::FriendGroupDelete { .. }
            | Self::SessionResetStart { .. }
            | Self::SessionReinitiate { .. }
            | Self::SessionResetAccept { .. }
            | Self::IdentityRotationAccept { .. }
            | Self::IdentityRotationDecline { .. }
            | Self::SessionResetDecline { .. } => Some(C::FriendsWrite),
            Self::Community(request) => request.agent_capability(),
            Self::DmInbox { .. }
            | Self::DmHistory { .. }
            | Self::GroupDmList
            | Self::GroupDmMessages { .. } => Some(C::ImHistory),
            Self::DmSend { .. }
            | Self::OutboxRetry { .. }
            | Self::OutboxDiscard { .. }
            | Self::GroupDmCreate { .. }
            | Self::GroupDmAccept { .. }
            | Self::GroupDmDecline { .. }
            | Self::GroupDmLeave { .. }
            | Self::GroupDmSend { .. }
            | Self::GroupDmTyping { .. }
            | Self::DmTyping { .. }
            | Self::DmMarkRead { .. } => Some(C::ImWrite),
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
            // Which keys the user trusts is the user's own decision.
            | Self::IdentityMarkVerified { .. }
            | Self::IdentityClearVerification { .. }
            // A commit runs the request the gate held, which passed this
            // caller's capability check when it was selected.
            | Self::IntentCommit { .. }
            | Self::IntentCancel { .. }
            // An undo's inverse passes this check on its own at the gate.
            | Self::Undo { .. }
            | Self::PrekeyStatus
            // Our name, our picture, the audit log and the system browser
            // are the user's own, never an agent's.
            | Self::ProfileSetNickname { .. }
            // Whether we tell friends what we read is the user's own.
            | Self::SetReadReceiptsEnabled { .. }
            | Self::ProfileSetAvatar { .. }
            | Self::AuditVerify
            | Self::AuditExport { .. }
            | Self::LinkOpen { .. }
            | Self::AgentRegister { .. }
            | Self::AgentRevoke { .. }
            | Self::PolicyReload => None,
        }
    }
}

impl CommunityRequest {
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
            | Self::ChannelList { .. }
            | Self::RoleList { .. }
            | Self::GameServerList { .. }
            | Self::ChannelMarkRead { .. }
            | Self::UnreadCounts { .. }
            | Self::OnboardingConfigGet { .. }
            | Self::WelcomeScreenGet { .. }
            | Self::AuditLog { .. }
            | Self::NotificationDefaultGet { .. }
            | Self::CommunityAvatarGet { .. }
            | Self::CommunityPolicyGet { .. }
            | Self::EventList { .. }
            | Self::EventAttendees { .. }
            | Self::BanList { .. }
            | Self::MemberBlockList { .. }
            | Self::AutomodRules { .. }
            | Self::ModReportList { .. }
            | Self::ChannelPins { .. }
            | Self::MessageSearch { .. }
            | Self::InviteList { .. } => Some(C::CommunitiesRead),
            Self::CommunityCreate { .. }
            | Self::CommunityJoin { .. }
            | Self::CommunityLeave { .. }
            | Self::CommunityUpdateInfo { .. }
            | Self::CommunityUpdateProfile { .. }
            | Self::CommunitySetAvatar { .. }
            | Self::CommunitySetBanner { .. }
            | Self::CommunityPolicySet { .. }
            | Self::MemberBlock { .. }
            | Self::MemberUnblock { .. }
            | Self::ChannelCreate { .. }
            | Self::ChannelDelete { .. }
            | Self::ChannelUpdate { .. }
            | Self::ChannelRename { .. }
            | Self::ChannelMove { .. }
            | Self::ChannelReorder { .. }
            | Self::ChannelSetTopic { .. }
            | Self::ChannelSetSlowmode { .. }
            | Self::MessagePin { .. }
            | Self::MessageUnpin { .. }
            | Self::AttachmentPin { .. }
            | Self::ChannelSetForumTags { .. }
            | Self::ChannelOverwriteSet { .. }
            | Self::ChannelOverwriteDelete { .. }
            | Self::CategoryCreate { .. }
            | Self::CategoryDelete { .. }
            | Self::CategoryRename { .. }
            | Self::CategoryReorder { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::RoleSelfAssign { .. }
            | Self::RoleSelfUnassign { .. }
            | Self::GameServerAdd { .. }
            | Self::NotificationDefaultSet { .. }
            | Self::GameServerRemove { .. }
            | Self::EventCreate { .. }
            | Self::EventEdit { .. }
            | Self::EventCancel { .. }
            | Self::EventDelete { .. }
            | Self::EventRsvp { .. }
            | Self::OnboardingConfigSet { .. }
            | Self::WelcomeScreenSet { .. }
            | Self::OnboardingSubmit { .. }
            | Self::OnboardingComplete { .. }
            | Self::InviteCreate { .. }
            | Self::InviteRevoke { .. } => Some(C::CommunitiesWrite),
            Self::CommunityApprove { .. }
            | Self::CommunityReject { .. }
            | Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. }
            | Self::TimeoutRemove { .. }
            | Self::AutomodRuleSet { .. }
            | Self::AutomodRuleDelete { .. } => Some(C::ModerationWrite),
            Self::ChannelMessages { .. }
            | Self::ChannelMessagesBefore { .. }
            | Self::ThreadList { .. }
            | Self::ThreadListActive { .. }
            | Self::ThreadMessages { .. }
            | Self::PollResults { .. } => Some(C::ChannelsHistory),
            Self::ChannelSend { .. }
            | Self::ChannelMessageEdit { .. }
            | Self::ChannelMessageDelete { .. }
            | Self::ChannelMessageSend { .. }
            | Self::ChannelMessageForward { .. }
            | Self::ChannelTyping { .. }
            | Self::ChannelSetActive { .. }
            | Self::ReactionAdd { .. }
            | Self::ReactionRemove { .. }
            | Self::ThreadCreate { .. }
            | Self::ThreadArchive { .. }
            | Self::ThreadUnarchive { .. }
            | Self::ThreadSend { .. }
            | Self::PollCreate { .. }
            | Self::PollVote { .. }
            | Self::PollClose { .. } => Some(C::ChatWrite),
            Self::ChannelMessageAdminDelete { .. } | Self::ChannelMessageBulkDelete { .. } => {
                Some(C::ModerationWrite)
            }
            // Channel keys are key material; ownership is the owner's own.
            Self::MekList { .. }
            | Self::MekRotate { .. }
            | Self::MekRequest { .. }
            | Self::CommunityTransferOwnership { .. } => None,
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
        assert_eq!(
            IpcRequest::Community(CommunityRequest::MekRotate {
                community: String::new(),
                channel: String::new()
            })
            .agent_capability(),
            None
        );
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
