//! State side-effects for subscription events.
//!
//! Extracts the ~10 event variants that mutate internal state (unread counts,
//! typing indicators, presence, voice participants). Called once per event
//! after the mechanical `into_event()` conversion, before dedup + emit.
//!
//! Pure state mutation — no I/O, no logging, no event emission.

use rekindle_types::subscription_events::{
    SubscriptionEvent, UnreadContext,
    ChannelMessageEvent, FriendEvent,
    TypingEvent, TypingContext,
    PresenceEvent, VoiceEvent, MembershipEvent,
};

use crate::state::SubscriptionState;

/// Apply state side-effects for an event. Returns additional events to emit
/// (e.g., `UnreadChanged` after incrementing a counter).
pub fn apply(state: &mut SubscriptionState, event: &SubscriptionEvent) -> Vec<SubscriptionEvent> {
    let mut extra = Vec::new();

    match event {
        // ── Unread: channel messages ────────────────────────────
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New { community, channel, .. }) => {
            let count = state.unread.increment_channel(community, channel);
            extra.push(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::Channel { community: community.clone(), channel: channel.clone() },
                count,
            });
        }

        // ── Unread: DMs ─────────────────────────────────────────
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived { peer_key, .. }) => {
            let count = state.unread.increment_dm(peer_key);
            extra.push(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::Dm { peer_key: peer_key.clone() },
                count,
            });
        }

        // ── Unread: friend requests ─────────────────────────────
        SubscriptionEvent::Friend(FriendEvent::RequestReceived { .. }) => {
            state.unread.friend_requests = state.unread.friend_requests.saturating_add(1);
            extra.push(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::FriendRequests,
                count: state.unread.friend_requests,
            });
        }

        // ── Typing: channel ─────────────────────────────────────
        SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Channel { community, channel }, who,
        }) => {
            state.typing.set_channel_typing(community, channel, who);
        }

        // ── Typing: DM ──────────────────────────────────────────
        SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Dm { .. }, who,
        }) => {
            state.typing.set_dm_typing(who);
        }
        SubscriptionEvent::Typing(TypingEvent::Stopped {
            context: TypingContext::Dm { .. }, who,
        }) => {
            state.typing.remove_dm_peer(who);
        }

        // ── Presence: community member ──────────────────────────
        SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community, pseudonym, status, game_name, game_id,
        }) => {
            state.presence.set_member(community, pseudonym, status, game_name.as_deref(), *game_id);
        }

        // ── Presence: friend/DM peer ────────────────────────────
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, status, game_name,
        }) => {
            state.presence.set_friend(peer_key, status, game_name.as_deref());
        }

        // ── Voice: join ─────────────────────────────────────────
        SubscriptionEvent::Voice(VoiceEvent::Joined { community, channel, pseudonym }) => {
            state.voice.join(community, channel, pseudonym, rekindle_utils::timestamp_ms());
        }

        // ── Voice: leave ────────────────────────────────────────
        SubscriptionEvent::Voice(VoiceEvent::Left { community, channel, pseudonym }) => {
            state.voice.leave(community, channel, pseudonym);
        }

        // ── Voice: mute/deafen ──────────────────────────────────
        SubscriptionEvent::Voice(VoiceEvent::MuteChanged { community, channel, target_pseudonym, muted }) => {
            state.voice.update_mute_deafen(community, channel, target_pseudonym, Some(*muted), None);
        }
        SubscriptionEvent::Voice(VoiceEvent::DeafenChanged { community, channel, target_pseudonym, deafened }) => {
            state.voice.update_mute_deafen(community, channel, target_pseudonym, None, Some(*deafened));
        }

        // ── Membership: remove presence on leave/kick/ban ───────
        SubscriptionEvent::Membership(
            MembershipEvent::Left { community, pseudonym }
            | MembershipEvent::Removed { community, pseudonym }
        ) => {
            state.presence.members.remove(&(community.clone(), pseudonym.clone()));
        }
        SubscriptionEvent::Membership(
            MembershipEvent::Kicked { community, target_pseudonym }
            | MembershipEvent::Banned { community, target_pseudonym }
        ) => {
            state.presence.members.remove(&(community.clone(), target_pseudonym.clone()));
        }

        // ── Friend: cleanup on unfriend ─────────────────────────
        SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) => {
            state.unread.remove_dm_peer(peer_key);
            state.typing.remove_dm_peer(peer_key);
            state.presence.remove_dm_peer(peer_key);
        }

        // All other events have no state side-effects
        _ => {}
    }

    extra
}
