//! `VoiceSessionEvent` → the bus's `VoiceEvent` (the desktop's
//! `event_mapping`, ported to the Host, plan E4.1 S5a2), and the session
//! state a window that subscribes after the join is handed.

use rekindle_types::subscription_events::{VoiceEvent, VoiceScope};
use rekindle_voice::{VoiceQualityCache, VoiceSessionEvent};

/// The session state key of the call we are in.
pub const SESSION_STATE: &str = "voice.session";
/// The session state key of our own place in it.
pub const SELF_STATE: &str = "voice.self";

/// `event` as the bus event for the session on `scope`; `ingress_drops`
/// is the Host's inbound drop count.
pub fn map(
    cache: &mut VoiceQualityCache,
    event: VoiceSessionEvent,
    scope: &VoiceScope,
    ingress_drops: u64,
) -> VoiceEvent {
    let scope = scope.clone();
    match event {
        VoiceSessionEvent::ConnectionQuality { .. } | VoiceSessionEvent::ReceiveStats { .. } => {
            cache.fold(&event);
            cache.connection_quality(scope, ingress_drops)
        }
        VoiceSessionEvent::UserJoined {
            peer_pubkey,
            display_name,
        } => VoiceEvent::Joined {
            scope,
            pseudonym: peer_pubkey,
            display_name: Some(display_name),
            // The route rides the signaling announcement.
            route_blob: None,
        },
        VoiceSessionEvent::UserLeft { peer_pubkey } => VoiceEvent::Left {
            scope,
            pseudonym: peer_pubkey,
        },
        VoiceSessionEvent::UserSpeaking {
            peer_pubkey,
            speaking,
        } => VoiceEvent::SpeakingChanged {
            scope,
            pseudonym: peer_pubkey,
            speaking,
        },
        VoiceSessionEvent::UserMuted { peer_pubkey, muted } => VoiceEvent::MuteChanged {
            scope,
            target_pseudonym: peer_pubkey,
            muted,
        },
        VoiceSessionEvent::DeviceChanged {
            device_type,
            reason,
        } => VoiceEvent::DeviceChanged {
            device_type,
            device_name: String::new(),
            reason,
        },
        VoiceSessionEvent::PacketsDropped { count } => VoiceEvent::PacketsDropped {
            scope,
            reason: "voice loop".into(),
            count,
        },
    }
}

/// The scope of a session on `channel_id` (a community's, or a 1:1 call's
/// whose channel id is the peer's key).
#[must_use]
pub fn scope_of(channel_id: &str, community_id: Option<&str>) -> VoiceScope {
    match community_id {
        Some(community) => VoiceScope::Community {
            community: community.to_string(),
            channel: channel_id.to_string(),
        },
        None => VoiceScope::Dm {
            peer_key: channel_id.to_string(),
        },
    }
}
