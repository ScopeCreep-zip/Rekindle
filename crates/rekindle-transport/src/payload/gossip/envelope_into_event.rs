//! `CommunityEnvelope` → `SubscriptionEvent`.
//!
//! The entry point the gossip ingress uses. It replaced
//! `GossipPayload::into_event`, which mapped a postcard near-copy of
//! `CommunityEnvelope` that only this track could read — 51 of the
//! canonical type's 72 control variants, several of them lossy
//! (`EventCreated` carried a `CommunityEvent` where the real one carries
//! an `EventInfo`).
//!
//! Mapping the canonical type directly means a desktop peer's
//! reactions, pins, events and voice signaling reach a daemon subscriber
//! instead of being logged and dropped, and there is one enum to add a
//! variant to rather than two.

use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, PresenceEvent, SubscriptionEvent, TypingContext, TypingEvent,
};

use super::into_event_control::control_into_event;

/// Convert a verified gossip envelope into a subscription event.
///
/// `None` for [`CommunityEnvelope::WatchRelay`]: it is not news about
/// the community, it is news about a *record*, and the handler acts on
/// it by fetching rather than by emitting. Returning an event for it
/// would surface Mutual Aid plumbing in the UI.
#[must_use]
pub fn envelope_into_event(
    envelope: CommunityEnvelope,
    community: &str,
    sender: &str,
) -> Option<SubscriptionEvent> {
    let c = || community.to_string();
    let s = || sender.to_string();

    Some(match envelope {
        CommunityEnvelope::MessageNotification {
            channel_id,
            message_id,
            sequence,
            timestamp,
            ..
        } => SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
            community: c(),
            channel: channel_id,
            message_id,
            sender_pseudonym: s(),
            sequence,
            timestamp,
            body: None,              // populated by the enrichment stage
            reply_to_sequence: None, // populated by the enrichment stage
        }),
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => SubscriptionEvent::Typing(TypingEvent::Started {
            context: TypingContext::Channel {
                community: c(),
                channel: channel_id,
            },
            who: pseudonym_key,
        }),
        CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status,
            game_info,
            ..
        } => SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
            community: c(),
            pseudonym: pseudonym_key,
            status,
            // Unpacked from `PresenceGameInfo` rather than dropped —
            // rich presence is the point of the field.
            game_name: game_info.as_ref().map(|g| g.game_name.clone()),
            game_id: game_info.as_ref().and_then(|g| g.game_id),
        }),
        CommunityEnvelope::Control(control) => {
            return control_into_event(control, community, sender)
        }
        CommunityEnvelope::WatchRelay { .. } => return None,
    })
}
