//! The community requests' `Debug` (plan E3.1), split by domain: each
//! domain file formats its own requests, and a message body, free text or
//! picture shows its length only.

mod audit;
mod channels;
mod events;
mod lifecycle;
mod moderation;
mod onboarding;
mod pins;
mod profile;
mod rich;
mod roles;

use super::super::CommunityRequest;

impl std::fmt::Debug for CommunityRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommunityCreate { .. }
            | Self::CommunityJoin { .. }
            | Self::CommunityLeave { .. }
            | Self::CommunityList
            | Self::CommunityViews
            | Self::CommunityMembers { .. }
            | Self::CommunityUpdateInfo { .. }
            | Self::CommunityInfo { .. }
            | Self::CommunityApprove { .. }
            | Self::CommunityReject { .. }
            | Self::CommunityPendingMembers { .. }
            | Self::CommunityTransferOwnership { .. }
            | Self::InviteCreate { .. }
            | Self::InviteList { .. }
            | Self::InviteRevoke { .. } => self.fmt_lifecycle(f),
            Self::ChannelPins { .. }
            | Self::MessagePin { .. }
            | Self::MessageUnpin { .. }
            | Self::AttachmentPin { .. }
            | Self::MessageSearch { .. } => self.fmt_pins(f),
            Self::CommunityUpdateProfile { .. }
            | Self::CommunitySetAvatar { .. }
            | Self::CommunitySetBanner { .. }
            | Self::CommunityAvatarGet { .. }
            | Self::CommunityPolicyGet { .. }
            | Self::CommunityPolicySet { .. } => self.fmt_profile(f),
            Self::ReactionAdd { .. }
            | Self::ReactionRemove { .. }
            | Self::ThreadCreate { .. }
            | Self::ThreadArchive { .. }
            | Self::ThreadUnarchive { .. }
            | Self::ThreadList { .. }
            | Self::ThreadListActive { .. }
            | Self::ThreadMessages { .. }
            | Self::ThreadSend { .. }
            | Self::PollCreate { .. }
            | Self::PollVote { .. }
            | Self::PollClose { .. }
            | Self::PollResults { .. } => self.fmt_rich(f),
            Self::ChannelSend { .. }
            | Self::ChannelList { .. }
            | Self::ChannelCreate { .. }
            | Self::ChannelDelete { .. }
            | Self::ChannelUpdate { .. }
            | Self::ChannelMessageEdit { .. }
            | Self::ChannelMessageDelete { .. }
            | Self::ChannelMessageForward { .. }
            | Self::ChannelMessageAdminDelete { .. }
            | Self::ChannelMessageBulkDelete { .. }
            | Self::ChannelTyping { .. }
            | Self::ChannelSetActive { .. }
            | Self::ChannelMessageSend { .. }
            | Self::ChannelMessages { .. }
            | Self::ChannelMessagesBefore { .. }
            | Self::MekList { .. }
            | Self::MekRotate { .. }
            | Self::MekRequest { .. } => self.fmt_channels(f),
            Self::ChannelRename { .. }
            | Self::ChannelMove { .. }
            | Self::ChannelReorder { .. }
            | Self::ChannelSetTopic { .. }
            | Self::ChannelSetSlowmode { .. }
            | Self::ChannelSetForumTags { .. }
            | Self::ChannelOverwriteSet { .. }
            | Self::ChannelOverwriteDelete { .. }
            | Self::CategoryCreate { .. }
            | Self::CategoryDelete { .. }
            | Self::CategoryRename { .. }
            | Self::CategoryReorder { .. } => self.fmt_channel_admin(f),
            Self::RoleList { .. }
            | Self::RoleCreate { .. }
            | Self::RoleUpdate { .. }
            | Self::RoleDelete { .. }
            | Self::RoleAssign { .. }
            | Self::RoleUnassign { .. }
            | Self::RoleSelfAssign { .. }
            | Self::RoleSelfUnassign { .. }
            | Self::GameServerList { .. }
            | Self::ChannelMarkRead { .. }
            | Self::UnreadCounts { .. }
            | Self::GameServerAdd { .. }
            | Self::GameServerRemove { .. } => self.fmt_roles_and_servers(f),
            Self::EventList { .. }
            | Self::EventAttendees { .. }
            | Self::EventCreate { .. }
            | Self::EventEdit { .. }
            | Self::EventCancel { .. }
            | Self::EventDelete { .. }
            | Self::EventRsvp { .. } => self.fmt_events(f),
            Self::OnboardingConfigGet { .. }
            | Self::OnboardingConfigSet { .. }
            | Self::WelcomeScreenGet { .. }
            | Self::WelcomeScreenSet { .. }
            | Self::OnboardingSubmit { .. }
            | Self::OnboardingComplete { .. } => self.fmt_onboarding(f),
            Self::AuditLog { .. }
            | Self::NotificationDefaultGet { .. }
            | Self::NotificationDefaultSet { .. } => self.fmt_audit(f),
            Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. }
            | Self::TimeoutRemove { .. }
            | Self::BanList { .. }
            | Self::MemberBlock { .. }
            | Self::MemberUnblock { .. }
            | Self::MemberBlockList { .. }
            | Self::AutomodRules { .. }
            | Self::AutomodRuleSet { .. }
            | Self::AutomodRuleDelete { .. }
            | Self::ModReportList { .. } => self.fmt_moderation(f),
        }
    }
}
