//! The community window's control gates (plan E3.2 S9), each a capability
//! the Host decides from the merged governance: a window shows every
//! control and disables the ones named in `CommunityView::denied` /
//! `GovernanceEvent::EffectivePermissionsChanged::denied` with the Host's
//! reason, never hides one and never reads permission bits itself (the
//! frontend computes no policy, plan E6.11; memory: never hide buttons).
//!
//! Which permission grants each one is the governance crate's
//! (`rekindle_governance::permissions::capabilities`), the same bits the
//! validator checks on the entry the control writes.

use serde::{Deserialize, Serialize};

/// One gated control family of the community window. On the wire a
/// camelCase string, also as a map key (`denied: Record<CommunityCapability,
/// string>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunityCapability {
    /// Overview edits (name, description, icon, banner, notification
    /// default): `CommunityMeta`.
    ManageCommunity,
    /// Channel list "+ Category", channel and category create, edit,
    /// archive and overwrites.
    ManageChannels,
    /// A channel's topic edit (`ChannelUpdated`).
    SetTopic,
    /// The community's game server list.
    ManageServers,
    /// The Roles tab: role definitions, assignments, archive.
    ManageRoles,
    /// Kick a member (`MemberRemoved`), approve or reject a knock.
    KickMembers,
    /// The Bans tab: ban and unban.
    BanMembers,
    /// Time a member out and lift it (`TimeoutEntry`).
    ModerateMembers,
    /// Mint an invite.
    CreateInvite,
    /// The Audit Log tab.
    ViewAuditLog,
    /// The Analytics tab.
    ViewInsights,
    /// The Emoji tab's manage controls: remove any expression, upload a
    /// sound.
    ManageExpressions,
    /// Upload an emoji or sticker.
    CreateExpressions,
    /// Edit, cancel or delete an event (`EventArchived`).
    ManageEvents,
    /// Schedule an event (`EventCreated`).
    CreateEvents,
    /// The Welcome tab (`WelcomeScreen`, `OnboardingConfig`).
    ManageWelcome,
    /// The AutoMod tab (`AutoModRule`).
    ManageAutomod,
    /// The Security tab (`CommunityPolicy`, `SegmentAdded`).
    ManageSecurity,
    /// Offer a successor community (`CommunitySuccessor`): a seated
    /// creator only (plan E3.2 S7s).
    OfferSuccessor,
}

impl CommunityCapability {
    /// Every capability, in wire order.
    pub const ALL: [Self; 19] = [
        Self::ManageCommunity,
        Self::ManageChannels,
        Self::SetTopic,
        Self::ManageServers,
        Self::ManageRoles,
        Self::KickMembers,
        Self::BanMembers,
        Self::ModerateMembers,
        Self::CreateInvite,
        Self::ViewAuditLog,
        Self::ViewInsights,
        Self::ManageExpressions,
        Self::CreateExpressions,
        Self::ManageEvents,
        Self::CreateEvents,
        Self::ManageWelcome,
        Self::ManageAutomod,
        Self::ManageSecurity,
        Self::OfferSuccessor,
    ];
}

/// The capabilities a member lacks, each with the Host's words naming what
/// grants it; a capability held is absent.
pub type DeniedCapabilities = std::collections::BTreeMap<CommunityCapability, String>;

/// One channel-scoped control of the community window (plan E3.2 S9),
/// decided with the channel's overwrites as the validator decides the
/// entry it writes: served per channel on `CommunityChannelView::denied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelCapability {
    /// A moderator's delete of another member's post and the bulk-delete
    /// entry (`AdminDelete`). The post author's rank is the Host's answer
    /// on the request, per post.
    ManageMessages,
    /// Play a soundboard sound in this voice channel (`SoundboardPlay`).
    UseSoundboard,
    /// Create a public thread or forum post.
    CreatePublicThreads,
    /// Reply in a thread or send the starter of a forum post.
    SendInThreads,
    /// Send a message in the channel's composer.
    SendMessages,
    /// Send without waiting for the channel's slowmode.
    BypassSlowmode,
    /// Raise a hand to speak in a stage channel.
    RequestToSpeak,
    /// Mention everyone or here in the channel.
    MentionEveryone,
}

impl ChannelCapability {
    /// Every channel capability, in wire order.
    pub const ALL: [Self; 8] = [
        Self::ManageMessages,
        Self::UseSoundboard,
        Self::CreatePublicThreads,
        Self::SendInThreads,
        Self::SendMessages,
        Self::BypassSlowmode,
        Self::RequestToSpeak,
        Self::MentionEveryone,
    ];
}

/// The channel capabilities a member lacks in one channel, each with the
/// Host's words; a capability held is absent.
pub type ChannelDeniedCapabilities = std::collections::BTreeMap<ChannelCapability, String>;
