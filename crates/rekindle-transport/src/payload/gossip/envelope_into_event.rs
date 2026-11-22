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

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_types::subscription_events::SubscriptionEvent;

use super::into_event_control::control_into_event;

/// Convert a verified gossip envelope into a subscription event.
///
/// `None` for a [`CommunityEnvelope::WatchRelay`]: it is not news about
/// the community, it is news about a *record*, and the handler acts on
/// it by fetching rather than by emitting. Returning an event for it
/// would surface Mutual Aid plumbing in the UI.
#[must_use]
pub fn envelope_into_event(
    envelope: CommunityEnvelope,
    community: &str,
    sender: &str,
) -> Option<SubscriptionEvent> {
    match envelope {
        CommunityEnvelope::Control(control) => control_into_event(*control, community, sender),
        // Channel typing is the Host's (plan E3.4 S9): only it holds the
        // text keys that name the channel behind the tag, and it lapses
        // the start itself (`operations::typing::channel_frame`). A
        // member's presence is its beat and card (plan E3.4 S2, S6): the
        // gossip `PresenceUpdate` (envelope arm @2) is obsolete.
        CommunityEnvelope::TypingIndicator { .. } | CommunityEnvelope::WatchRelay { .. } => None,
    }
}
