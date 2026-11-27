//! Channel-scoped controls served by the Host.

use serde::{Deserialize, Serialize};

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
    /// Reply to a thread, including the starter of a forum post.
    SendInThreads,
}

impl ChannelCapability {
    /// Every channel capability, in wire order.
    pub const ALL: [Self; 4] = [
        Self::ManageMessages,
        Self::UseSoundboard,
        Self::CreatePublicThreads,
        Self::SendInThreads,
    ];
}

/// The channel capabilities a member lacks in one channel, each with the
/// Host's words; a capability held is absent.
pub type ChannelDeniedCapabilities = std::collections::BTreeMap<ChannelCapability, String>;
