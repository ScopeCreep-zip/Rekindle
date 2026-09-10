//! Subscription event types — the complete set of signals emitted to consumers.
//!
//! Each domain has its own module with a self-describing enum. The top-level
//! [`SubscriptionEvent`] wraps them all. Consumers pattern-match on the
//! outer enum to route by domain, then on the inner enum to handle specifics.
//!
//! Every [`ControlPayload`], [`GossipPayload`], and [`DmPayload`] variant
//! maps to exactly one event. No Veilid types cross this boundary.

mod call;
mod channel;
mod crypto;
mod friend;
mod governance;
mod membership;
mod network;
mod notification;
mod presence;
mod social;
mod system;
mod typing;
mod voice;

pub use call::{CallEvent, DirectCallInfo};
pub use channel::ChannelMessageEvent;
pub use crypto::{CryptoEvent, PqBundleKind};
pub use friend::FriendEvent;
pub use governance::GovernanceEvent;
pub use membership::MembershipEvent;
pub use network::NetworkEvent;
pub use notification::NotificationEvent;
pub use presence::{GameActivity, PresenceEvent, PresenceSnapshot};
pub use social::SocialEvent;
pub use system::SystemEvent;
pub use typing::{TypingContext, TypingEvent};
pub use voice::{LinkMeasurement, VoiceEvent, VoiceParticipant, VoiceScope};

use serde::{Deserialize, Serialize};

/// Top-level subscription event. Every signal the subscription manager
/// emits is one of these. Consumers receive them via
/// `SubscriptionManager::subscribe()`.
/// Externally tagged and `camelCase` on purpose. This type crosses two
/// wires: postcard on the daemon IPC socket (`ipc/framing.rs`), which
/// cannot decode an internally/adjacently tagged enum, and JSON to the
/// Tauri webview, which wants the project's camelCase convention.
/// External tagging is the one representation that satisfies both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubscriptionEvent {
    /// Channel message lifecycle (new, edited, deleted, pinned, thread).
    ChannelMessage(ChannelMessageEvent),
    /// Typing indicators (channel and DM).
    Typing(TypingEvent),
    /// Member presence changes (online, away, game activity).
    Presence(PresenceEvent),
    /// Community membership lifecycle (join, leave, ban, kick, timeout, roles).
    Membership(MembershipEvent),
    /// Friend lifecycle (request, accept, reject, remove).
    Friend(FriendEvent),
    /// Cryptographic key events (MEK rotation, MEK request, MEK transfer).
    Crypto(CryptoEvent),
    /// Voice channel activity (join, leave, mute, roster).
    Voice(VoiceEvent),
    /// Community governance changes (metadata, channels, roles, invites, permissions).
    Governance(GovernanceEvent),
    /// Social features (reactions, pins, threads, events, game servers).
    Social(SocialEvent),
    /// Call signalling — ring, answer, decline, media state. Not a
    /// message: it shared the desktop's chat channel only because that
    /// was the one channel available to put it on.
    Call(CallEvent),
    /// Device-level notifications to surface to the user — an
    /// incoming call, an app update, a message worth a sound.
    Notification(NotificationEvent),
    /// Network and infrastructure events (attachment, routes, watches).
    Network(NetworkEvent),
    /// System-level signals (announcements, raid alerts, kicked, sync, bootstrap).
    System(SystemEvent),
    /// Unread count changed for a specific context.
    UnreadChanged { context: UnreadContext, count: u32 },
}

/// Context for unread count changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnreadContext {
    Channel { community: String, channel: String },
    Dm { peer_key: String },
    FriendRequests,
}

// ── Subscription filtering ─────────────────────────────────────────────

/// Type-safe event category for subscription filtering.
///
/// Maps 1:1 to `SubscriptionEvent` discriminants. No string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCategory {
    ChannelMessage,
    Typing,
    Presence,
    Membership,
    Friend,
    Crypto,
    Voice,
    Governance,
    Social,
    Call,
    Notification,
    Network,
    System,
    UnreadChanged,
}

impl SubscriptionEvent {
    /// The category of this event.
    pub fn category(&self) -> EventCategory {
        match self {
            Self::ChannelMessage(_) => EventCategory::ChannelMessage,
            Self::Typing(_) => EventCategory::Typing,
            Self::Presence(_) => EventCategory::Presence,
            Self::Membership(_) => EventCategory::Membership,
            Self::Friend(_) => EventCategory::Friend,
            Self::Crypto(_) => EventCategory::Crypto,
            Self::Voice(_) => EventCategory::Voice,
            Self::Governance(_) => EventCategory::Governance,
            Self::Social(_) => EventCategory::Social,
            Self::Call(_) => EventCategory::Call,
            Self::Notification(_) => EventCategory::Notification,
            Self::Network(_) => EventCategory::Network,
            Self::System(_) => EventCategory::System,
            Self::UnreadChanged { .. } => EventCategory::UnreadChanged,
        }
    }

    /// Extract the community governance key from this event, if it has one.
    /// Returns None for global events (friend, network, unread).
    pub fn community(&self) -> Option<&str> {
        match self {
            Self::ChannelMessage(e) => match e {
                ChannelMessageEvent::New { community, .. }
                | ChannelMessageEvent::Edited { community, .. }
                | ChannelMessageEvent::Deleted { community, .. } => Some(community),
                // Direct-conversation events belong to a peer, not a
                // community.
                ChannelMessageEvent::DirectMessageReceived { .. }
                | ChannelMessageEvent::DirectMessageAcknowledged { .. }
                | ChannelMessageEvent::DirectConversationInvited { .. }
                | ChannelMessageEvent::ConversationFocusRequested { .. } => None,
            },
            Self::Typing(e) => match e {
                TypingEvent::Started { context, .. } | TypingEvent::Stopped { context, .. } => {
                    match context {
                        TypingContext::Channel { community, .. } => Some(community),
                        TypingContext::Dm { .. } => None,
                    }
                }
            },
            Self::Presence(e) => match e {
                PresenceEvent::CommunityMemberChanged { community, .. } => Some(community),
                // Neither is community-scoped: a DM peer's presence and
                // our own belong to the device, not to one community.
                PresenceEvent::FriendChanged { .. } | PresenceEvent::SelfChanged { .. } => None,
            },
            Self::Membership(e) => Some(match e {
                MembershipEvent::JoinRequested { community, .. }
                | MembershipEvent::JoinAccepted { community, .. }
                | MembershipEvent::JoinRejected { community, .. }
                | MembershipEvent::Joined { community, .. }
                | MembershipEvent::Left { community, .. }
                | MembershipEvent::Removed { community, .. }
                | MembershipEvent::Kicked { community, .. }
                | MembershipEvent::Banned { community, .. }
                | MembershipEvent::Unbanned { community, .. }
                | MembershipEvent::TimedOut { community, .. }
                | MembershipEvent::TimeoutRemoved { community, .. }
                | MembershipEvent::TimeoutStatusChanged { community, .. }
                | MembershipEvent::RolesChanged { community, .. }
                | MembershipEvent::OnboardingCompleted { community, .. }
                | MembershipEvent::OnboardingAnswersSubmitted { community, .. }
                | MembershipEvent::MembersRefreshed { community }
                | MembershipEvent::MemberDiscovered { community, .. }
                | MembershipEvent::JoinProgress { community, .. } => community,
            }),
            Self::Crypto(e) => match e {
                CryptoEvent::MekRotated { community, .. }
                | CryptoEvent::MekRequested { community, .. }
                | CryptoEvent::MekTransferred { community, .. }
                | CryptoEvent::AdminKeypairGranted { community }
                | CryptoEvent::SlotKeypairGranted { community, .. } => Some(community),
                // PqBundlePublished is a profile-level event (subkey 5 of
                // the user's identity record), not community-scoped.
                CryptoEvent::PqBundlePublished { .. } => None,
            },
            // `None` for a DM call and for device changes, both of
            // which belong to no community.
            Self::Voice(e) => e.community(),
            Self::Governance(e) => Some(e.community()),
            Self::Social(e) => Some(match e {
                SocialEvent::ReactionAdded { community, .. }
                | SocialEvent::ReactionRemoved { community, .. }
                | SocialEvent::MessagePinned { community, .. }
                | SocialEvent::MessageUnpinned { community, .. }
                | SocialEvent::ThreadCreated { community, .. }
                | SocialEvent::ThreadMessagePosted { community, .. }
                | SocialEvent::ThreadArchiveChanged { community, .. }
                | SocialEvent::EventCreated { community, .. }
                | SocialEvent::EventUpdated { community, .. }
                | SocialEvent::EventDeleted { community, .. }
                | SocialEvent::EventRsvpChanged { community, .. }
                | SocialEvent::EventReminder { community, .. }
                | SocialEvent::GameServerAdded { community, .. }
                | SocialEvent::GameServerRemoved { community, .. } => community,
            }),
            Self::System(e) => match e {
                SystemEvent::Announcement { community, .. } => community.as_deref(),
                SystemEvent::RaidAlert { community, .. }
                | SystemEvent::RaidDetected { community, .. }
                | SystemEvent::AutoModAlert { community, .. }
                | SystemEvent::ChannelLockdown { community, .. }
                | SystemEvent::Kicked { community }
                | SystemEvent::BootstrapRequested { community, .. }
                | SystemEvent::BootstrapReceived { community }
                | SystemEvent::SyncRequested { community, .. }
                | SystemEvent::SyncReceived { community, .. } => Some(community),
                // Audit chain breakage is a local-device event, not community-scoped.
                SystemEvent::AuditChainBroken { .. } => None,
            },
            // Not community-scoped: a friend event, a network state
            // change and an unread bump all belong to the device, not
            // to one community.
            // A notification names its community inside the payload
            // where it has one, but the family as a whole is
            // device-scoped — a call and an app update belong to
            // no community.
            // A call belongs to its participants, not to a community.
            Self::Call(_)
            | Self::Network(_)
            | Self::Notification(_)
            | Self::UnreadChanged { .. }
            | Self::Friend(_) => None,
        }
    }
}

/// Subscription filter for event routing.
///
/// Clients register filters to receive only events they care about.
/// Connections with zero filters receive zero events (fail closed).
/// Maximum 64 filters per connection to bound memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    /// Event categories to match. None = all categories.
    pub categories: Option<Vec<EventCategory>>,
    /// Community scope (governance key). None = all communities + global events.
    /// Some(key) = only events for that community + global events.
    pub community_scope: Option<String>,
}

/// Maximum filters per connection.
pub const MAX_FILTERS_PER_CONNECTION: usize = 64;

impl SubscriptionFilter {
    /// Match all events.
    pub fn all() -> Self {
        Self {
            categories: None,
            community_scope: None,
        }
    }

    /// Match all events for a specific community (plus global events).
    pub fn community(gov_key: String) -> Self {
        Self {
            categories: None,
            community_scope: Some(gov_key),
        }
    }

    /// Match specific event categories across all communities.
    pub fn categories(cats: Vec<EventCategory>) -> Self {
        Self {
            categories: Some(cats),
            community_scope: None,
        }
    }

    /// Check if this filter matches an event.
    pub fn matches(&self, event: &SubscriptionEvent) -> bool {
        // Category check
        if let Some(ref cats) = self.categories {
            if !cats.contains(&event.category()) {
                return false;
            }
        }

        // Community scope check
        if let Some(ref scope) = self.community_scope {
            // `if let` rather than `match`: only the Some arm acts.
            // Global events (friend, network, unread) have no community
            // and pass every community filter.
            if let Some(community) = event.community() {
                if community != scope {
                    return false;
                }
            }
        }

        true
    }
}
