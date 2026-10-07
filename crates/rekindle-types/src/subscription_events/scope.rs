//! Which part of the app an event belongs to.
//!
//! [`SubscriptionEvent::scope`] names the conversation, community or call
//! an event is about, so every frontend routes it the same way: the
//! desktop maps a scope to the windows that show it, a TUI to a pane, the
//! daemon's router to the connections that subscribed. The mapping from
//! scope to a particular UI stays in each frontend; the scope itself is
//! policy and lives here.
//!
//! The match is exhaustive with no wildcard, so a new event variant does
//! not compile until it has a scope.

use super::{
    ChannelMessageEvent, CryptoEvent, FriendEvent, MembershipEvent, PresenceEvent, SocialEvent,
    SubscriptionEvent, SystemEvent, TypingContext, TypingEvent, UnreadContext, VoiceScope,
};

/// The context an event belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventScope {
    /// This device and identity as a whole: own presence, network state,
    /// device-level notifications, invites into new conversations.
    Device,
    /// One community, by governance key.
    Community(String),
    /// A community being joined. No view of it exists yet, so it belongs
    /// to wherever the join was started.
    CommunityJoin(String),
    /// A 1:1 relationship with a peer, by identity public key: the DM
    /// conversation, the friendship, the peer's presence, a DM call's media.
    Peer(String),
    /// A conversation held in its own DHT record (a record-backed DM or a
    /// group DM), by record key.
    Conversation(String),
    /// A call, by call id.
    Call(String),
}

impl EventScope {
    /// A direct conversation: a 1:1 DM is keyed by its peer, anything else
    /// by its record.
    fn direct(peer_key: &str, conversation_id: &str) -> Self {
        if conversation_id == peer_key {
            Self::Peer(peer_key.to_owned())
        } else {
            Self::Conversation(conversation_id.to_owned())
        }
    }

    fn voice(scope: &VoiceScope) -> Self {
        match scope {
            VoiceScope::Community { community, .. } => Self::Community(community.clone()),
            VoiceScope::Dm { peer_key } => Self::Peer(peer_key.clone()),
        }
    }
}

impl SubscriptionEvent {
    /// The context this event belongs to.
    #[must_use]
    pub fn scope(&self) -> EventScope {
        match self {
            Self::ChannelMessage(e) => channel_message_scope(e),
            Self::Typing(
                TypingEvent::Started { context, .. } | TypingEvent::Stopped { context, .. },
            ) => match context {
                TypingContext::Channel { community, .. } => {
                    EventScope::Community(community.clone())
                }
                TypingContext::Dm { peer_key } => EventScope::Peer(peer_key.clone()),
            },
            Self::Presence(e) => match e {
                PresenceEvent::CommunityMemberChanged { community, .. } => {
                    EventScope::Community(community.clone())
                }
                PresenceEvent::FriendChanged { peer_key, .. } => EventScope::Peer(peer_key.clone()),
                PresenceEvent::SelfChanged { .. } => EventScope::Device,
            },
            Self::Membership(e) => match e {
                MembershipEvent::JoinProgress { community, .. } => {
                    EventScope::CommunityJoin(community.clone())
                }
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
                | MembershipEvent::MemberDiscovered { community, .. } => {
                    EventScope::Community(community.clone())
                }
            },
            Self::Friend(e) => EventScope::Peer(friend_peer(e).to_owned()),
            Self::Crypto(e) => match e {
                CryptoEvent::MekRotated { community, .. }
                | CryptoEvent::MekRequested { community, .. }
                | CryptoEvent::MekTransferred { community, .. }
                | CryptoEvent::AdminKeypairGranted { community }
                | CryptoEvent::SlotKeypairGranted { community, .. } => {
                    EventScope::Community(community.clone())
                }
                CryptoEvent::PqBundlePublished { .. } => EventScope::Device,
            },
            // Device changes (`scope() == None`) are machine-wide.
            Self::Voice(e) => e.scope().map_or(EventScope::Device, EventScope::voice),
            Self::Governance(e) => EventScope::Community(e.community().to_owned()),
            Self::Social(e) => EventScope::Community(social_community(e).to_owned()),
            Self::Call(e) => EventScope::Call(e.call_id().to_owned()),
            // A notification is about something, but surfacing it is the
            // device's job: one OS toast, one inbox, whatever the source.
            Self::Notification(_) | Self::Network(_) => EventScope::Device,
            Self::System(e) => match e {
                SystemEvent::Announcement { community, .. } => community
                    .clone()
                    .map_or(EventScope::Device, EventScope::Community),
                SystemEvent::RaidAlert { community, .. }
                | SystemEvent::RaidDetected { community, .. }
                | SystemEvent::AutoModAlert { community, .. }
                | SystemEvent::ChannelLockdown { community, .. }
                | SystemEvent::Kicked { community }
                | SystemEvent::BootstrapRequested { community, .. }
                | SystemEvent::BootstrapReceived { community }
                | SystemEvent::SyncRequested { community, .. }
                | SystemEvent::SyncReceived { community, .. } => {
                    EventScope::Community(community.clone())
                }
                SystemEvent::AuditChainBroken { .. } => EventScope::Device,
            },
            Self::UnreadChanged { context, .. } => match context {
                UnreadContext::Channel { community, .. } => {
                    EventScope::Community(community.clone())
                }
                UnreadContext::Dm { peer_key } => EventScope::Peer(peer_key.clone()),
                UnreadContext::FriendRequests => EventScope::Device,
            },
        }
    }
}

fn channel_message_scope(e: &ChannelMessageEvent) -> EventScope {
    match e {
        ChannelMessageEvent::New { community, .. }
        | ChannelMessageEvent::Edited { community, .. }
        | ChannelMessageEvent::Deleted { community, .. } => EventScope::Community(community.clone()),
        ChannelMessageEvent::DirectMessageReceived {
            peer_key,
            conversation_id,
            ..
        } => EventScope::direct(peer_key, conversation_id),
        ChannelMessageEvent::DirectMessageAcknowledged {
            conversation_id, ..
        } => EventScope::Peer(conversation_id.clone()),
        // An invitation names a conversation we do not show yet; it is
        // answered from the device-wide inbox.
        ChannelMessageEvent::DirectConversationInvited { .. }
        // Bringing a conversation forward is the device's decision.
        | ChannelMessageEvent::ConversationFocusRequested { .. } => EventScope::Device,
    }
}

fn friend_peer(e: &FriendEvent) -> &str {
    match e {
        FriendEvent::RequestReceived { from_key, .. } => from_key,
        FriendEvent::RequestAcknowledged { peer_key }
        | FriendEvent::Accepted { peer_key, .. }
        | FriendEvent::Rejected { peer_key }
        | FriendEvent::Added { peer_key, .. }
        | FriendEvent::Removed { peer_key }
        | FriendEvent::RemoveAcknowledged { peer_key }
        | FriendEvent::ProfileKeyRotated { peer_key, .. }
        | FriendEvent::NicknameChanged { peer_key, .. } => peer_key,
    }
}

fn social_community(e: &SocialEvent) -> &str {
    match e {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription_events::{CallEvent, NotificationEvent, VoiceEvent};

    #[test]
    fn direct_messages_scope_by_peer_or_record() {
        let one_to_one =
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
                peer_key: "pk".into(),
                timestamp: 1,
                sender_name: None,
                body: None,
                decryption_failed: false,
                automod_blurred: false,
                conversation_id: "pk".into(),
                server_message_id: None,
                reply_to_id: None,
            });
        assert_eq!(one_to_one.scope(), EventScope::Peer("pk".into()));
        let group = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key: "pk".into(),
            timestamp: 1,
            sender_name: None,
            body: None,
            decryption_failed: false,
            automod_blurred: false,
            conversation_id: "VLD0:rec".into(),
            server_message_id: None,
            reply_to_id: None,
        });
        assert_eq!(group.scope(), EventScope::Conversation("VLD0:rec".into()));
        let ack =
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageAcknowledged {
                message_id: 7,
                conversation_id: "pk".into(),
            });
        assert_eq!(ack.scope(), EventScope::Peer("pk".into()));
    }

    #[test]
    fn joins_calls_voice_and_notifications() {
        let join = SubscriptionEvent::Membership(MembershipEvent::MembersRefreshed {
            community: "c".into(),
        });
        assert_eq!(join.scope(), EventScope::Community("c".into()));
        let call = SubscriptionEvent::Call(CallEvent::Ringing {
            call_id: "id".into(),
        });
        assert_eq!(call.scope(), EventScope::Call("id".into()));
        let dm_voice = SubscriptionEvent::Voice(VoiceEvent::LocalJoined {
            scope: VoiceScope::Dm {
                peer_key: "pk".into(),
            },
        });
        assert_eq!(dm_voice.scope(), EventScope::Peer("pk".into()));
        let alert = SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            title: "t".into(),
            body: "b".into(),
        });
        assert_eq!(alert.scope(), EventScope::Device);
        let nick = SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
            peer_key: "pk".into(),
            nickname: None,
        });
        assert_eq!(nick.scope(), EventScope::Peer("pk".into()));
    }
}
