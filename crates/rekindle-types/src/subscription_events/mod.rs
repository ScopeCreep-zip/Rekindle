//! Subscription event types — the complete set of signals emitted to consumers.
//!
//! Each domain has its own module with a self-describing enum. The top-level
//! [`SubscriptionEvent`] wraps them all. Consumers pattern-match on the
//! outer enum to route by domain, then on the inner enum to handle specifics.
//!
//! Every [`ControlPayload`], [`GossipPayload`], and [`DmPayload`] variant
//! maps to exactly one event. No Veilid types cross this boundary.

mod channel;
mod crypto;
mod friend;
mod governance;
mod membership;
mod network;
mod presence;
mod social;
mod system;
mod typing;
mod voice;

pub use channel::ChannelMessageEvent;
pub use crypto::{CryptoEvent, PqBundleKind};
pub use friend::FriendEvent;
pub use governance::GovernanceEvent;
pub use membership::MembershipEvent;
pub use network::NetworkEvent;
pub use presence::PresenceEvent;
pub use social::SocialEvent;
pub use system::SystemEvent;
pub use typing::{TypingContext, TypingEvent};
pub use voice::VoiceEvent;

use serde::{Deserialize, Serialize};

/// Top-level subscription event. Every signal the subscription manager
/// emits is one of these. Consumers receive them via
/// `SubscriptionManager::subscribe()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
                ChannelMessageEvent::DirectMessageReceived { .. } => None,
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
                PresenceEvent::FriendChanged { .. } => None,
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
                | MembershipEvent::OnboardingAnswersSubmitted { community, .. } => community,
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
            Self::Voice(e) => Some(match e {
                VoiceEvent::Joined { community, .. }
                | VoiceEvent::Left { community, .. }
                | VoiceEvent::ModeChanged { community, .. }
                | VoiceEvent::MuteChanged { community, .. }
                | VoiceEvent::DeafenChanged { community, .. }
                | VoiceEvent::RosterUpdated { community, .. } => community,
            }),
            Self::Governance(e) => Some(match e {
                GovernanceEvent::MetadataChanged { community }
                | GovernanceEvent::ChannelsChanged { community }
                | GovernanceEvent::RolesChanged { community }
                | GovernanceEvent::BansChanged { community }
                | GovernanceEvent::InvitesChanged { community }
                | GovernanceEvent::ChannelPermissionsChanged { community, .. }
                | GovernanceEvent::GovernanceSubkeyUpdated { community, .. } => community,
            }),
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
                | SystemEvent::ChannelLockdown { community, .. }
                | SystemEvent::Kicked { community }
                | SystemEvent::BootstrapRequested { community, .. }
                | SystemEvent::BootstrapReceived { community }
                | SystemEvent::SyncRequested { community, .. }
                | SystemEvent::SyncReceived { community, .. } => Some(community),
                // Audit chain breakage is a local-device event, not community-scoped.
                SystemEvent::AuditChainBroken { .. } => None,
            },
            Self::Network(_) => None,
            Self::UnreadChanged { .. } => None,
            Self::Friend(_) => None,
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
            match event.community() {
                Some(community) => {
                    if community != scope {
                        return false;
                    }
                }
                None => {
                    // Global events (friend, network, unread) pass community filters
                    // — they're relevant regardless of community scope.
                }
            }
        }

        true
    }
}
