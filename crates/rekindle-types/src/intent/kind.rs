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
    SetNickname,
    SetStatus,
    SetStatusMessage,
    UpdateCommunityPresence,
    UpdateCommunityProfile,
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
    // Routine: Games
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
    /// Create a group DM: an invite goes to every member (plan E2.6).
    CreateGroupDm,
    /// Leave a group DM: every other member rotates its key.
    LeaveGroupDm,
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
    // TrustChanging: Key trust
    AcceptSessionReset,
    AcceptIdentityRotation,
    ResetSignalSession,
    ReinitiateSession,
    RotateMek,
    // TrustChanging: Exposure settings
    RegisterWithPushRelay,
    SetLinkPreviewsEnabled,
    SetReadReceiptsEnabled,
    SetPresencePolicy,
    VolunteerRelay,
    // TrustChanging: Outbound files and links
    OpenExternalUrl,
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
    TransferCommunityOwnership,
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
    // Protected: Not yet built
    ArmQuickWipe,
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
            | Self::SetNickname
            | Self::SetStatus
            | Self::SetStatusMessage
            | Self::UpdateCommunityPresence
            | Self::UpdateCommunityProfile
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
            | Self::LaunchGameToServer
            | Self::CancelInvite
            | Self::CancelRequest
            | Self::Logout
            | Self::Lock
            | Self::Shutdown
            | Self::PolicyReload
            | Self::UpdateChannel
            | Self::SetGamePresence
            | Self::ClearGamePresence => IntentClass::Routine,
            Self::BlockUser
            | Self::DeclineDmInvite
            | Self::DeclineSessionReset
            | Self::DeclineIdentityRotation
            | Self::DismissDeepLink
            | Self::MuteCallerTemp
            | Self::RejectRequest
            | Self::RemoveFriend
            | Self::RotateFriendToken
            | Self::LeaveCommunity
            | Self::CreateGroupDm
            | Self::LeaveGroupDm
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
            | Self::RevokeAgent => IntentClass::Consequential,
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
            | Self::AcceptSessionReset
            | Self::AcceptIdentityRotation
            | Self::ResetSignalSession
            | Self::ReinitiateSession
            | Self::RotateMek
            | Self::RegisterWithPushRelay
            | Self::SetLinkPreviewsEnabled
            | Self::SetReadReceiptsEnabled
            | Self::SetPresencePolicy
            | Self::VolunteerRelay
            | Self::OpenExternalUrl
            | Self::UploadAttachment
            | Self::GeneratePairingQrSvg
            | Self::StartPairingSession
            | Self::ApproveJoinRequest
            | Self::MarkVerified
            | Self::ClearVerification => IntentClass::TrustChanging,
            Self::BulkDeleteChannelMessages
            | Self::TransferCommunityOwnership
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
            | Self::ArmQuickWipe => IntentClass::Protected,
        }
    }
}
