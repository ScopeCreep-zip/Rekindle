//! Phase 14.r split — pure `VoiceSessionEvent → VoiceEvent` mapping +
//! the small `emit_local_joined` fan-out.
//!
//! Extracted from the trait impl so deps_impl stays under the 500
//! LoC cap. No state access — pure value mapping; the fan-out takes
//! `AppHandle` by reference and uses it only for emit calls.

use std::sync::Arc;

use rekindle_voice::VoiceSessionEvent;

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceScope};

use crate::state::AppState;
use tauri::Manager;

/// Phase 5 — fold a quality/receive-stats event into the cache and
/// build the merged UI event. Returns `None` for every other variant.
pub(super) fn merge_quality_event(
    state: &Arc<AppState>,
    event: &rekindle_voice::VoiceSessionEvent,
    scope: &VoiceScope,
) -> Option<VoiceEvent> {
    let mut cache = state.voice_quality_cache.lock();
    if !cache.fold(event) {
        return None;
    }
    Some(
        cache.connection_quality(
            scope.clone(),
            state
                .voice_ingress_drops_total
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
    )
}

pub(super) fn map(event: VoiceSessionEvent, scope: &VoiceScope) -> VoiceEvent {
    match event {
        VoiceSessionEvent::UserJoined {
            peer_pubkey,
            display_name,
        } => VoiceEvent::Joined {
            scope: scope.clone(),
            pseudonym: peer_pubkey,
            display_name: Some(display_name),
            // The session-level join fires on the first received voice
            // packet, by which point the route is already resolved —
            // the blob rides the gossip announcement instead.
            route_blob: None,
        },
        VoiceSessionEvent::UserLeft { peer_pubkey } => VoiceEvent::Left {
            scope: scope.clone(),
            pseudonym: peer_pubkey,
        },
        VoiceSessionEvent::UserSpeaking {
            peer_pubkey,
            speaking,
        } => VoiceEvent::SpeakingChanged {
            scope: scope.clone(),
            pseudonym: peer_pubkey,
            speaking,
        },
        VoiceSessionEvent::UserMuted { peer_pubkey, muted } => VoiceEvent::MuteChanged {
            scope: scope.clone(),
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
            scope: scope.clone(),
            reason: "voice loop".into(),
            count,
        },
        VoiceSessionEvent::ConnectionQuality { .. } | VoiceSessionEvent::ReceiveStats { .. } => {
            unreachable!(
                "quality/receive-stats are merged + emitted in emit_voice_event before map()"
            )
        }
    }
}

/// The session-state keys of the call we are in and of our own entry in
/// it, in the host's journal (plan C7.29).
pub(crate) const SESSION_STATE: &str = "voice.session";
pub(crate) const SELF_STATE: &str = "voice.self";

/// Fan-out for `VoiceSessionDeps::emit_local_joined`: 4 sequential
/// `voice-event` emits (LocalJoined + UserJoined + ConnectionQuality
/// + UserSpeaking) matching the pre-Phase-14 `emit_join_events` body.
/// Send/receive loops update each event independently as state
/// changes.
pub(super) fn emit_local_joined_impl(
    app: &tauri::AppHandle,
    channel_id: &str,
    community_id: Option<&str>,
    public_key: &str,
    display_name: &str,
) {
    // The call's shape, built here rather than read from the engine:
    // this fires as the session comes up, so the handle may not be
    // installed yet. `community_id` is exactly what decided the old
    // `active_call_type` string, so the two carry the same fact.
    let scope = match community_id {
        Some(community) => VoiceScope::Community {
            community: community.to_string(),
            channel: channel_id.to_string(),
        },
        None => VoiceScope::Dm {
            peer_key: channel_id.to_string(),
        },
    };

    // The call we are in and our own place in it are session state (plan
    // C7.29): a window that subscribes after the join is handed them, then
    // the roster, then the live changes.
    let session_state = [
        (
            SESSION_STATE,
            VoiceEvent::LocalJoined {
                scope: scope.clone(),
            },
        ),
        (
            SELF_STATE,
            VoiceEvent::Joined {
                scope: scope.clone(),
                pseudonym: public_key.to_string(),
                display_name: Some(display_name.to_string()),
                // This is us joining our own call; nobody needs a route to
                // reach us from this event.
                route_blob: None,
            },
        ),
    ];
    if let Some(host) = app
        .try_state::<crate::state::SharedState>()
        .and_then(|state| state.host.current())
    {
        for (key, event) in session_state {
            host.publish_state(key, SubscriptionEvent::Voice(event));
        }
    } else {
        tracing::warn!("voice join not published: the host has not started");
    }
    for event in [
        VoiceEvent::ConnectionQuality {
            scope: scope.clone(),
            quality: "good".to_string(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_key_drops: 0,
            ingress_drops: 0,
            // Join-time initial state: no peer has reported yet, so
            // there is nothing measured to show.
            link: None,
        },
        VoiceEvent::SpeakingChanged {
            scope: scope.clone(),
            pseudonym: public_key.to_string(),
            speaking: false,
        },
    ] {
        crate::event_dispatch::emit_subscription(app, &SubscriptionEvent::Voice(event));
    }
}
