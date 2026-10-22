//! The community requests' `Debug` (plan E3.1), split by domain: each
//! domain file formats its own requests, and a message body, free text or
//! picture shows its length only.

mod channels;
mod events;
mod lifecycle;
mod moderation;
mod onboarding;
mod profile;
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
            Self::CommunityUpdateProfile { .. }
            | Self::CommunitySetAvatar { .. }
            | Self::CommunitySetBanner { .. }
            | Self::CommunityAvatarGet { .. }
            | Self::CommunityPolicyGet { .. }
            | Self::CommunityPolicySet { .. } => self.fmt_profile(f),
            Self::ChannelSend { .. }
            | Self::ChannelList { .. }
            | Self::ChannelCreate { .. }
            | Self::ChannelDelete { .. }
            | Self::ChannelUpdate { .. }
            | Self::ChannelHistory { .. }
            | Self::MekList { .. }
            | Self::MekRotate { .. }
            | Self::MekRequest { .. } => self.fmt_channels(f),
            Self::ChannelRename { .. }
            | Self::ChannelMove { .. }
            | Self::ChannelReorder { .. }
            | Self::ChannelSetTopic { .. }
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
            Self::Kick { .. }
            | Self::Ban { .. }
            | Self::Unban { .. }
            | Self::Timeout { .. }
            | Self::TimeoutRemove { .. }
            | Self::BanList { .. } => self.fmt_moderation(f),
        }
    }
}
