//! Payload → SubscriptionEvent conversions.
//!
//! `dm_to_event` is implemented inline (DmPayload has no into_event on the type).
//! `gossip_to_event` delegates to `GossipPayload::into_event` which lives on
//! the type in `rekindle-types` with the exhaustive 52-variant ControlPayload match.

use rekindle_types::dm_payload::DmPayload;
use rekindle_types::gossip_payload::GossipPayload;
use rekindle_types::subscription_events::{
    SubscriptionEvent, TypingEvent, TypingContext, FriendEvent, PresenceEvent,
};

/// Convert a DM payload into a SubscriptionEvent.
pub fn dm_to_event(payload: DmPayload, sender_key: &str) -> SubscriptionEvent {
    match payload {
        DmPayload::Typing { typing } => {
            if typing {
                SubscriptionEvent::Typing(TypingEvent::Started {
                    context: TypingContext::Dm { peer_key: sender_key.into() },
                    who: sender_key.into(),
                })
            } else {
                SubscriptionEvent::Typing(TypingEvent::Stopped {
                    context: TypingContext::Dm { peer_key: sender_key.into() },
                    who: sender_key.into(),
                })
            }
        }
        DmPayload::FriendRequestAck => {
            SubscriptionEvent::Friend(FriendEvent::RequestAcknowledged {
                peer_key: sender_key.into(),
            })
        }
        DmPayload::Unfriend => {
            SubscriptionEvent::Friend(FriendEvent::Removed {
                peer_key: sender_key.into(),
            })
        }
        DmPayload::UnfriendAck => {
            SubscriptionEvent::Friend(FriendEvent::RemoveAcknowledged {
                peer_key: sender_key.into(),
            })
        }
        DmPayload::ProfileKeyRotated { new_profile_dht_key } => {
            SubscriptionEvent::Friend(FriendEvent::ProfileKeyRotated {
                peer_key: sender_key.into(),
                new_profile_dht_key,
            })
        }
        DmPayload::PresenceUpdate { status, game_info } => {
            let status_str = match status {
                0 => "online", 1 => "away", 2 => "busy",
                3 => "offline", 4 => "invisible", _ => "unknown",
            };
            SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
                peer_key: sender_key.into(),
                status: status_str.into(),
                game_name: game_info.map(|g| g.game_name),
            })
        }
    }
}

/// Convert a gossip payload into a SubscriptionEvent.
///
/// Delegates to `GossipPayload::into_event` which has the exhaustive
/// match over all GossipPayload and ControlPayload variants.
pub fn gossip_to_event(
    payload: GossipPayload,
    community: &str,
    sender: &str,
) -> SubscriptionEvent {
    payload.into_event(community, sender)
}
