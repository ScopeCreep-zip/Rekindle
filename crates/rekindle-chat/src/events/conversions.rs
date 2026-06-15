//! Payload → SubscriptionEvent conversions.
//!
//! DM payloads use `DmPayload::into_event` directly on the type
//! (in `rekindle-types/src/dm_payload.rs`).
//!
//! `gossip_to_event` delegates to `GossipPayload::into_event` which lives on
//! the type in `rekindle-types` with the exhaustive ControlPayload match.

use rekindle_types::gossip_payload::GossipPayload;
use rekindle_types::subscription_events::SubscriptionEvent;

/// Convert a gossip payload into a SubscriptionEvent.
pub fn gossip_to_event(
    payload: GossipPayload,
    community: &str,
    sender: &str,
) -> SubscriptionEvent {
    payload.into_event(community, sender)
}
