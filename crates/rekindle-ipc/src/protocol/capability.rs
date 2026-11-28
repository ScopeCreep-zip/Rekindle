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
            // A community's page reads the community; every other subject
            // reads our friends (design §2.2).
            Self::InfoviewGet {
                subject: crate::protocol::InfoviewSubject::Community { .. },
                ..
            } => Some(C::CommunitiesRead),
            Self::IdentityShow
            | Self::PresenceGet
            | Self::GameTotalsGet
            | Self::HiddenGamesGet
            | Self::InstalledGamesGet
            | Self::ProfileBioGet
            | Self::AvatarGet { .. } => {
                Some(C::IdentityRead)
            }
            Self::NetworkStatus | Self::NetworkPeers => Some(C::NetworkRead),
            Self::Subscribe { .. } | Self::Unsubscribe { .. } => Some(C::EventsRead),
            Self::FriendList
            | Self::FriendRequests
            | Self::FriendInviteList
            | Self::FriendBlockedList
            | Self::FriendCode
            | Self::FriendGameTotalsGet { .. }
            | Self::FriendBioGet { .. }
            | Self::ActivityList { .. }
            | Self::InfoviewGet { .. }
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
            Self::Transfer(request) => request.agent_capability(),
            Self::Servers(request) => request.agent_capability(),
            Self::Lobby(request) => Some(request.agent_capability()),
            Self::GroupDm(request) => Some(request.agent_capability()),
            Self::DmInbox { .. }
            | Self::DmHistory { .. }
            | Self::DisappearingGet { .. } => Some(C::ImHistory),
            Self::DmSend { .. }
            | Self::GameInviteSend { .. }
            | Self::OutboxRetry { .. }
            | Self::OutboxDiscard { .. }
            | Self::DisappearingSet { .. }
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
            | Self::PermissionCatalogue
            | Self::Shutdown
            | Self::IdentityCreate { .. }
            | Self::IdentityList
            | Self::IdentityExport { .. }
            | Self::IdentityImport { .. }
            | Self::IdentityRotate { .. }
            | Self::IdentityDelete { .. }
            | Self::IdentityWipe
            | Self::QuickWipeArm { .. }
            | Self::QuickWipeDisarm
            | Self::QuickWipeTrigger
            // Which keys the user trusts is the user's own decision.
            | Self::IdentityMarkVerified { .. }
            | Self::IdentityClearVerification { .. }
            // A commit runs the request the gate held, which passed this
            // caller's capability check when it was selected.
            | Self::IntentCommit { .. }
            | Self::IntentListHeld
            | Self::IntentCancel { .. }
            // An undo's inverse passes this check on its own at the gate.
            | Self::Undo { .. }
            | Self::PrekeyStatus
            // Our name, our picture, the audit log and the system browser
            // are the user's own, never an agent's.
            | Self::ProfileSetNickname { .. }
            | Self::ProfileSetBio { .. }
            // Whether we tell friends what we read is the user's own.
            | Self::SetReadReceiptsEnabled { .. }
            // So is whether friends see our game time.
            | Self::SetShareGameTotals { .. }
            // And whether we scan for games, which friends see, and which
            // games are never shown.
            | Self::SetGameDetection { .. }
            | Self::SetShareCurrentGame { .. }
            // And when auto-away shows us Away.
            | Self::SetAutoAway { .. }
            // And whether we keep a log of what friends did.
            | Self::SetKeepActivityLog { .. }
            | Self::GameHide { .. }
            | Self::GameUnhide { .. }
            | Self::ProfileSetAvatar { .. }
            // What the app looks like, and what shows before unlock, is
            // the user's own.
            | Self::SkinList
            | Self::SkinImport { .. }
            | Self::SkinRemove { .. }
            | Self::SkinSelect { .. }
            | Self::SkinSetLogin { .. }
            | Self::SkinKeep { .. }
            | Self::SkinGet { .. }
            | Self::SkinFile { .. }
            | Self::SkinFrames
            | Self::SkinValidateDraft { .. }
            | Self::SkinDraftStart { .. }
            | Self::SkinExportDraft { .. }
            | Self::SkinDraftCheck { .. }
            | Self::SkinExportSchema { .. }
            | Self::RunSkinAction { .. }
            | Self::SkinSelectionSet { .. }
            | Self::SkinApplyFailed { .. }
            | Self::AuditVerify
            | Self::AuditExport { .. }
            | Self::LinkOpen { .. }
            // No agent starts a process on the user's machine.
            | Self::GameLaunch { .. }
            | Self::AgentRegister { .. }
            | Self::AgentRevoke { .. }
            // One local search reads friends, members, direct history and
            // the user's held files at once, more than any one read
            // capability grants.
            | Self::LocalSearch { .. }
            | Self::PolicyReload => None,
        }
    }
}

#[cfg(test)]
#[path = "capability_tests.rs"]
mod tests;
