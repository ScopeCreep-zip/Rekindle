//! Every user action that is an intent, and its base class (plan U1.1;
//! `realignment/r9-intents.md` §8). Generated from r9's table and then
//! owned here: a new kind does not compile until it is classified.

use serde::{Deserialize, Serialize};

use super::IntentClass;

/// A user action that is not a pure read or plumbing call. Exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntentKind {
    // Routine: Messaging (send)
    SendChannelMessage,
    SendDmMessage,
    /// Retry an unsettled or failed send under its client id (plan E2.5).
    RetrySend,
    SendMessage,
    SendThreadMessage,
    SendVoiceMessage,
    // Routine: Messaging (light actions)
    AddReaction,
    ArchiveThread,
    CreateThread,
    EditChannelMessage,
    PinAttachment,
    PinMessage,
    RemoveReaction,
    RsvpEvent,
    SendCallReaction,
    SetEventRsvp,
    UnarchiveThread,
    UnpinMessage,
    VotePoll,
    // Routine: Own presence and profile
    SetAvatar,
    SetBio,
    SetNickname,
    SetStatus,
    SetStatusMessage,
    UpdateCommunityProfile,
    SetCommunityDisplayName,
    // Routine: Voice and call controls
    AcceptDmCall,
    AcceptGroupCall,
    DeclineDmCall,
    DeclineGroupCall,
    EndDmCall,
    EndGroupCall,
    JoinVoiceChannel,
    LeaveVoice,
    PlaySoundboard,
    RequestToSpeak,
    SetAudioDevices,
    SetDeafen,
    SetInputChannels,
    SetMute,
    SetVideoDevice,
    SetVoiceMode,
    StartDmCall,
    StartGroupCall,
    StartNativeVideo,
    StopNativeVideo,
    // Routine: Notification and local preferences
    SetChannelNotificationLevel,
    SetCommunityDefaultNotificationLevel,
    SetDoNotDisturb,
    SetNotificationSound,
    SetPreferences,
    SetQuietHours,
    WriteSyncPreferences,
    // Routine: Friend organisation
    CreateFriendGroup,
    DeleteFriendGroup,
    MoveFriendToGroup,
    RenameFriendGroup,
    SetFriendNickname,
    // Routine: Community content you create
    AddGameServer,
    ClosePoll,
    CreateCategory,
    CreateChannel,
    CreateEvent,
    CreatePoll,
    CreateRole,
    EditEvent,
    EditRole,
    MarkOnboardingComplete,
    MoveChannel,
    RenameCategory,
    RenameChannel,
    ReorderCategories,
    ReorderChannels,
    SelfAssignRole,
    SelfUnassignRole,
    SetChannelForumTags,
    SetChannelTopic,
    SetCommunityAvatar,
    SetCommunityBanner,
    SetOnboardingConfig,
    SetWelcomeScreen,
    SubmitOnboardingAnswers,
    UpdateCommunityInfo,
    UploadEmoji,
    UploadSoundboardSound,
    UploadSticker,
    // Routine: Files
    DownloadAttachment,
    RevealDownloadedAttachment,
    /// Pause a running download (plan E7.2 G12, `TransferStop`).
    PauseDownload,
    /// Forget a transfer and, when asked, delete its saved file (plan
    /// E7.2 G12, `TransferRemove`): one kind per request name, so the
    /// stricter class, since it may delete a file.
    RemoveDownload,
    // Routine: Games
    /// Launch a game we have installed, or to one of our own favourite
    /// servers (`GameLaunch`, lead decision (50)).
    LaunchGameToServer,
    // Routine: Outgoing requests you can withdraw
    CancelInvite,
    CancelRequest,
    // Routine: Session
    Logout,
    // Consequential: Relationships (protective)
    BlockUser,
    DeclineDmInvite,
    DeclineSessionReset,
    DeclineIdentityRotation,
    DismissDeepLink,
    MuteCallerTemp,
    RejectRequest,
    // Consequential: Relationships (others notified)
    RemoveFriend,
    // Consequential: Withdrawing reach you gave out
    RotateFriendToken,
    // Consequential: Membership
    LeaveCommunity,
    RemoveCoCreator,
    /// Co-sign the re-admission of a member's fork-answering device key
    /// (ADR 0022 rules 7 and 8): every member then counts that key's ops.
    ReadmitDevice,
    CancelCommunityCreate,
    /// Withdraw our standing transfer offer (plan E3.2 S7s.4, decision
    /// (26)).
    WithdrawCommunityTransfer,
    /// Create a group DM: an invite goes to every member (plan E2.6).
    CreateGroupDm,
    /// Invite friends into a group DM we created (plan E7.2 G8).
    InviteToGroupDm,
    /// Leave a group DM: every other member rotates its key.
    LeaveGroupDm,
    /// Set a DM's or group DM's disappearing-message timer (plan E7.1):
    /// every other member is told.
    SetDisappearingTimer,
    // Consequential: Moderation (reversible)
    RemoveTimeout,
    TimeoutMember,
    // Consequential: Moderation and admin (reversible)
    AssignRole,
    BanMember,
    CancelEvent,
    DeleteAutomodRule,
    DeleteChannelOverwrite,
    ExpandCommunitySegment,
    RemoveCommunityMember,
    RemoveGameServer,
    RespondToSpeakRequest,
    RevokeCommunityInvite,
    ServerDeafenMember,
    ServerMuteMember,
    SetAutomodRule,
    SetChannelOverwrite,
    SetCommunityPolicy,
    SetSlowmode,
    UnassignRole,
    // Consequential: Deleting content (single, own or admin)
    AdminDeleteChannelMessage,
    DeleteChannelMessage,
    DeleteEmoji,
    /// Discard a failed send: it settles failed and leaves its stream.
    DiscardSend,
    DeleteEvent,
    // Consequential: Deleting containers
    DeleteCategory,
    DeleteChannel,
    DeleteRole,
    // Consequential: Revoking help you offered
    RevokeRelay,
    UnregisterWithPushRelay,
    // Consequential: Crossing an audience
    ForwardChannelMessage,
    // TrustChanging: Admitting a person
    AcceptDmInvite,
    AcceptRequest,
    AddFriend,
    AddFriendFromInvite,
    UnbanMember,
    UnblockUser,
    // TrustChanging: Giving out reach
    CreateCommunityInvite,
    GenerateInvite,
    // TrustChanging: Admitting a community or link
    ConfirmDeepLink,
    CreateCommunity,
    JoinCommunity,
    /// Follow a successor both sides link: join its new anchor, keeping
    /// the old community (ADR 0011, plan E3.2 S7s.4).
    FollowCommunitySuccessor,
    /// Accept a transfer offered to us: collect the other creators' links,
    /// create the successor and name it (plan E3.2 S7s.4).
    AcceptCommunityTransfer,
    /// Answer a transfer's link request asking us, with our creator link
    /// into the successor (lead decision (46)).
    AnswerSuccessorLink,
    AnswerCreatorInvite,
    // TrustChanging: Key trust
    AcceptSessionReset,
    AcceptIdentityRotation,
    ResetSignalSession,
    ReinitiateSession,
    RotateMek,
    /// Retire one of our device keys (ADR 0022 rules 4 and 5): changes
    /// which keys every member trusts to sign as us, permanently.
    RetireDevice,
    /// Enrol this device in a community with a fresh device key (ADR 0022
    /// rules 1-4, lead decision 63): `p` endorses it, and every member
    /// then trusts it to sign as us.
    EnrolDevice,
    // TrustChanging: Exposure settings
    RegisterWithPushRelay,
    SetLinkPreviewsEnabled,
    SetReadReceiptsEnabled,
    ShareGameTotals,
    ShareCurrentGame,
    SetPresencePolicy,
    VolunteerRelay,
    // TrustChanging: Outbound files and links
    OpenExternalUrl,
    /// Launch a game to a server whose address a peer chose (lead decision
    /// (50)): our IP address reaches that host.
    JoinGameServer,
    /// Launch a group DM's lobby we host (plan E7.2 G6, SK2-G §5): its
    /// server address, often our own IP, reaches every member.
    LaunchGameLobby,
    /// Ask the Servers view's servers what they are running (plan E7.2 G4,
    /// `ServersRefresh`): the user's own Refresh, as Xfire's. Our IP
    /// address reaches every server asked, addresses friends and
    /// communities supplied among them, so it is `JoinGameServer`'s case
    /// (r9 §7.2: metadata leaving the app) and the review lists them.
    RefreshServers,
    UploadAttachment,
    // TrustChanging: Starting device pairing
    GeneratePairingQrSvg,
    StartPairingSession,
    // Destructive: Irreversible loss for others
    BulkDeleteChannelMessages,
    // Protected: Identity and key material
    AcceptPairingCode,
    AuditExport,
    DeleteIdentity,
    // Protected: Unlock (the passphrase entry is itself the proof; no added gate)
    CreateIdentity,
    Login,
    // Routine: Session (daemon)
    Lock,
    Shutdown,
    // Routine: Daemon administration (daemon)
    PolicyReload,
    // Routine: Community content you create (daemon)
    UpdateChannel,
    /// Rename a group DM we created (plan E7.2 G7): every member sees the
    /// new name; renaming back undoes it.
    RenameGroupDm,
    /// Open a game lobby in a group DM for our shown game (plan E7.2 G6).
    HostGameLobby,
    /// Join another member's open lobby (plan E7.2 G6); leaving undoes it.
    JoinGameLobby,
    /// Leave a lobby we joined (plan E7.2 G6).
    LeaveGameLobby,
    /// Close a lobby we host (plan E7.2 G6, Xfire's "Stop Hosting").
    CloseGameLobby,
    // Routine: Own presence and profile (daemon)
    SetGamePresence,
    ClearGamePresence,
    // Consequential: Membership (daemon)
    RejectJoinRequest,
    // Consequential: Revoking help you offered (daemon)
    RevokeAgent,
    // TrustChanging: Admitting a person (daemon)
    ApproveJoinRequest,
    // Destructive: Irreversible loss for others (daemon)
    /// Re-create the community under a new anchor whose `GenesisCert`
    /// names this one as `predecessor`, and offer it to the members (ADR
    /// 0011 "Successor community", plan E3.2 S7s.4): moves creator
    /// authority to a new creator set for everyone who follows.
    PublishCommunitySuccessor,
    /// Offer the community to a member: once accepted, a successor whose
    /// creator set drops us and adds that member (plan E3.2 S7s.4).
    OfferCommunityTransfer,
    /// Delete a community we were removed from: its kept history and
    /// every local trace (plan E3.2 S9 R3-5; Element's "Forget this room").
    ForgetCommunity,
    // Protected: Identity and key material (daemon)
    ExportIdentity,
    ImportIdentity,
    RotateIdentity,
    WipeAllData,
    RegisterAgent,
    // Destructive: Not yet built
    DeleteCommunity,
    DeleteAccount,
    // Protected: Not yet built
    ExportKeys,
    ViewRecoveryMaterial,
    // TrustChanging: Key trust (plan E2.3, safety numbers)
    MarkVerified,
    ClearVerification,
    // The quick wipe (plan E7.1): arming and disarming are Protected; the
    // panic shortcut's trigger is one Routine act with no review (r9 §7.8,
    // F12), refused by its handler while unarmed.
    ArmQuickWipe,
    DisarmQuickWipe,
    TriggerQuickWipe,
    // Skins (plan E7.7.2)
    /// Import a skin package into the identity's library: TrustChanging,
    /// as it brings third-party pictures and sounds into every window.
    ImportSkin,
    /// Set the device login skin: Routine for a built-in; a library
    /// package is raised to TrustChanging by
    /// `Escalation::VisibleBeforeUnlock`.
    SetLoginSkin,
    /// Remove a package from the library.
    RemoveSkin,
}

impl IntentKind {
    /// The class before escalations (r9 §8). No wildcard.
    #[must_use]
    pub const fn base_class(self) -> IntentClass {
        match self {
            Self::SendChannelMessage
            | Self::SendDmMessage
            | Self::RetrySend
            | Self::SendMessage
            | Self::SendThreadMessage
            | Self::SendVoiceMessage
            | Self::AddReaction
            | Self::ArchiveThread
            | Self::CreateThread
            | Self::EditChannelMessage
            | Self::PinAttachment
            | Self::PinMessage
            | Self::RemoveReaction
            | Self::RsvpEvent
            | Self::SendCallReaction
            | Self::SetEventRsvp
            | Self::UnarchiveThread
            | Self::UnpinMessage
            | Self::VotePoll
            | Self::SetAvatar
            | Self::SetBio
            | Self::SetNickname
            | Self::SetStatus
            | Self::SetStatusMessage
            | Self::UpdateCommunityProfile
            | Self::SetCommunityDisplayName
            | Self::AcceptDmCall
            | Self::AcceptGroupCall
            | Self::DeclineDmCall
            | Self::DeclineGroupCall
            | Self::EndDmCall
            | Self::EndGroupCall
            | Self::JoinVoiceChannel
            | Self::LeaveVoice
            | Self::PlaySoundboard
            | Self::RequestToSpeak
            | Self::SetAudioDevices
            | Self::SetDeafen
            | Self::SetInputChannels
            | Self::SetMute
            | Self::SetVideoDevice
            | Self::SetVoiceMode
            | Self::StartDmCall
            | Self::StartGroupCall
            | Self::StartNativeVideo
            | Self::StopNativeVideo
            | Self::SetChannelNotificationLevel
            | Self::SetCommunityDefaultNotificationLevel
            | Self::SetDoNotDisturb
            | Self::SetNotificationSound
            | Self::SetPreferences
            | Self::SetQuietHours
            | Self::WriteSyncPreferences
            | Self::CreateFriendGroup
            | Self::DeleteFriendGroup
            | Self::MoveFriendToGroup
            | Self::RenameFriendGroup
            | Self::SetFriendNickname
            | Self::AddGameServer
            | Self::ClosePoll
            | Self::CreateCategory
            | Self::CreateChannel
            | Self::CreateEvent
            | Self::CreatePoll
            | Self::CreateRole
            | Self::EditEvent
            | Self::EditRole
            | Self::MarkOnboardingComplete
            | Self::MoveChannel
            | Self::RenameCategory
            | Self::RenameChannel
            | Self::ReorderCategories
            | Self::ReorderChannels
            | Self::SelfAssignRole
            | Self::SelfUnassignRole
            | Self::SetChannelForumTags
            | Self::SetChannelTopic
            | Self::SetCommunityAvatar
            | Self::SetCommunityBanner
            | Self::SetOnboardingConfig
            | Self::SetWelcomeScreen
            | Self::SubmitOnboardingAnswers
            | Self::UpdateCommunityInfo
            | Self::UploadEmoji
            | Self::UploadSoundboardSound
            | Self::UploadSticker
            | Self::DownloadAttachment
            | Self::RevealDownloadedAttachment
            | Self::PauseDownload
            | Self::LaunchGameToServer
            | Self::CancelInvite
            | Self::CancelRequest
            | Self::Logout
            | Self::Lock
            | Self::Shutdown
            | Self::PolicyReload
            | Self::UpdateChannel
            | Self::RenameGroupDm
            | Self::HostGameLobby
            | Self::JoinGameLobby
            | Self::LeaveGameLobby
            | Self::CloseGameLobby
            | Self::SetGamePresence
            | Self::ClearGamePresence
            | Self::SetLoginSkin
            | Self::TriggerQuickWipe => IntentClass::Routine,
            Self::BlockUser
            | Self::RemoveDownload
            | Self::DeclineDmInvite
            | Self::DeclineSessionReset
            | Self::DeclineIdentityRotation
            | Self::DismissDeepLink
            | Self::MuteCallerTemp
            | Self::RejectRequest
            | Self::RemoveFriend
            | Self::RotateFriendToken
            | Self::LeaveCommunity
            | Self::RemoveCoCreator
            | Self::ReadmitDevice
            | Self::CancelCommunityCreate
            | Self::WithdrawCommunityTransfer
            | Self::CreateGroupDm
            | Self::InviteToGroupDm
            | Self::LeaveGroupDm
            | Self::SetDisappearingTimer
            | Self::RemoveTimeout
            | Self::TimeoutMember
            | Self::AssignRole
            | Self::BanMember
            | Self::CancelEvent
            | Self::DeleteAutomodRule
            | Self::DeleteChannelOverwrite
            | Self::ExpandCommunitySegment
            | Self::RemoveCommunityMember
            | Self::RemoveGameServer
            | Self::RespondToSpeakRequest
            | Self::RevokeCommunityInvite
            | Self::ServerDeafenMember
            | Self::ServerMuteMember
            | Self::SetAutomodRule
            | Self::SetChannelOverwrite
            | Self::SetCommunityPolicy
            | Self::SetSlowmode
            | Self::UnassignRole
            | Self::AdminDeleteChannelMessage
            | Self::DeleteChannelMessage
            | Self::DeleteEmoji
            | Self::DiscardSend
            | Self::DeleteEvent
            | Self::DeleteCategory
            | Self::DeleteChannel
            | Self::DeleteRole
            | Self::RevokeRelay
            | Self::UnregisterWithPushRelay
            | Self::ForwardChannelMessage
            | Self::RejectJoinRequest
            | Self::RevokeAgent
            | Self::RemoveSkin => IntentClass::Consequential,
            Self::AcceptDmInvite
            | Self::AcceptRequest
            | Self::AddFriend
            | Self::AddFriendFromInvite
            | Self::UnbanMember
            | Self::UnblockUser
            | Self::CreateCommunityInvite
            | Self::GenerateInvite
            | Self::ConfirmDeepLink
            | Self::CreateCommunity
            | Self::JoinCommunity
            | Self::FollowCommunitySuccessor
            | Self::AcceptCommunityTransfer
            | Self::AnswerSuccessorLink
            | Self::AnswerCreatorInvite
            | Self::AcceptSessionReset
            | Self::AcceptIdentityRotation
            | Self::ResetSignalSession
            | Self::ReinitiateSession
            | Self::RotateMek
            | Self::RetireDevice
            | Self::EnrolDevice
            | Self::RegisterWithPushRelay
            | Self::SetLinkPreviewsEnabled
            | Self::SetReadReceiptsEnabled
            | Self::ShareGameTotals
            | Self::ShareCurrentGame
            | Self::SetPresencePolicy
            | Self::VolunteerRelay
            | Self::OpenExternalUrl
            | Self::JoinGameServer
            | Self::LaunchGameLobby
            | Self::RefreshServers
            | Self::UploadAttachment
            | Self::GeneratePairingQrSvg
            | Self::StartPairingSession
            | Self::ApproveJoinRequest
            | Self::MarkVerified
            | Self::ClearVerification
            | Self::ImportSkin => IntentClass::TrustChanging,
            Self::BulkDeleteChannelMessages
            | Self::PublishCommunitySuccessor
            | Self::OfferCommunityTransfer
            | Self::ForgetCommunity
            | Self::DeleteCommunity
            | Self::DeleteAccount => IntentClass::Destructive,
            Self::AcceptPairingCode
            | Self::AuditExport
            | Self::DeleteIdentity
            | Self::CreateIdentity
            | Self::Login
            | Self::ExportIdentity
            | Self::ImportIdentity
            | Self::RotateIdentity
            | Self::WipeAllData
            | Self::RegisterAgent
            | Self::ExportKeys
            | Self::ViewRecoveryMaterial
            | Self::ArmQuickWipe
            | Self::DisarmQuickWipe => IntentClass::Protected,
        }
    }
}
