//! State side-effects for subscription events.
//!
//! Extracts the ~10 event variants that mutate internal state (friend requests,
//! typing indicators, presence, voice participants). Called once per event
//! after the mechanical `into_event()` conversion, before dedup + emit.
//!
//! Pure state mutation — no I/O, no logging, no event emission.

use rekindle_types::subscription_events::{
    FriendEvent, MembershipEvent, PresenceEvent, SubscriptionEvent, TypingContext, TypingEvent,
    UnreadContext, VoiceEvent, VoiceScope,
};

use crate::state::SubscriptionState;

/// Apply state side-effects for an event. Returns additional events to emit
/// (e.g., `UnreadChanged` after incrementing a counter).
pub fn apply(
    state: &mut SubscriptionState,
    event: &SubscriptionEvent,
    now_ms: u64,
) -> Vec<SubscriptionEvent> {
    let mut extra = Vec::new();

    match event {
        // ── Unread: friend requests ─────────────────────────────
        SubscriptionEvent::Friend(FriendEvent::RequestReceived { .. }) => {
            state.friend_requests = state.friend_requests.saturating_add(1);
            extra.push(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::FriendRequests,
                count: state.friend_requests,
            });
        }

        // ── Typing: channel ─────────────────────────────────────
        SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Channel { community, channel },
            who,
        }) => {
            state.typing.set_channel_typing(community, channel, who);
        }

        // ── Presence: community member ──────────────────────────
        SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community,
            pseudonym,
            snapshot,
        }) => {
            state.presence.set_member(community, pseudonym, snapshot);
        }

        // ── Presence: friend/DM peer ────────────────────────────
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged { peer_key, snapshot }) => {
            state.presence.set_friend(peer_key, snapshot);
        }

        // ── Voice: join ─────────────────────────────────────────
        // `VoiceState` is keyed by (community, channel) and has only
        // ever held community calls — gossip was its only source. A DM
        // call has no community, so it is skipped here rather than
        // given a sentinel community id to sit under.
        SubscriptionEvent::Voice(VoiceEvent::Joined {
            scope: VoiceScope::Community { community, channel },
            pseudonym,
            ..
        }) => {
            state.voice.join(community, channel, pseudonym, now_ms);
        }

        // ── Voice: leave ────────────────────────────────────────
        SubscriptionEvent::Voice(VoiceEvent::Left {
            scope: VoiceScope::Community { community, channel },
            pseudonym,
        }) => {
            state.voice.leave(community, channel, pseudonym);
        }

        // ── Voice: mute/deafen ──────────────────────────────────
        SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
            scope: VoiceScope::Community { community, channel },
            target_pseudonym,
            muted,
        }) => {
            state.voice.update_mute_deafen(
                community,
                channel,
                target_pseudonym,
                Some(*muted),
                None,
            );
        }
        SubscriptionEvent::Voice(VoiceEvent::DeafenChanged {
            scope: VoiceScope::Community { community, channel },
            target_pseudonym,
            deafened,
        }) => {
            state.voice.update_mute_deafen(
                community,
                channel,
                target_pseudonym,
                None,
                Some(*deafened),
            );
        }

        // ── Membership: remove presence on leave/kick/ban ───────
        SubscriptionEvent::Membership(
            MembershipEvent::Left {
                community,
                pseudonym,
            }
            | MembershipEvent::Removed {
                community,
                pseudonym,
            },
        ) => {
            state
                .presence
                .members
                .remove(&(community.clone(), pseudonym.clone()));
        }
        SubscriptionEvent::Membership(
            MembershipEvent::Kicked {
                community,
                target_pseudonym,
            }
            | MembershipEvent::Banned {
                community,
                target_pseudonym,
            },
        ) => {
            state
                .presence
                .members
                .remove(&(community.clone(), target_pseudonym.clone()));
        }

        // ── Friend: cleanup on unfriend ─────────────────────────
        SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) => {
            state.presence.remove_dm_peer(peer_key);
        }

        // All other events have no state side-effects.
        //
        // `PresenceEvent::SelfChanged` lands here deliberately: our own
        // presence is not a peer row. Nothing in `PresenceState` tracks
        // us, and inserting ourselves would put us in
        // `community_members()`. Frontends read it off the event.
        _ => {}
    }

    extra
}
