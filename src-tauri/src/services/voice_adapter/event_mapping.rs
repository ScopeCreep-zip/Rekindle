//! Phase 14.r split — pure `VoiceSessionEvent → VoiceEvent` mapping +
//! the small `emit_local_joined` fan-out.
//!
//! Extracted from the trait impl so deps_impl stays under the 500
//! LoC cap. No state access — pure value mapping; the fan-out takes
//! `AppHandle` by reference and uses it only for emit calls.

use std::sync::Arc;

use rekindle_voice::VoiceSessionEvent;

use crate::channels::VoiceEvent;
use crate::state::AppState;

/// Phase B — sync the per-call video session aggregator when a peer
/// appears or disappears from the voice session. Other `VoiceSessionEvent`
/// variants (speaking, mute, etc.) carry no video-session-relevant
/// signal and are left untouched.
///
/// The `(community_id, channel_id)` pair is read from the active voice
/// engine — a peer-join VoiceSessionEvent that fires while no engine is
/// joined means the receive loop saw a stray packet from an old session;
/// safe to ignore (the join helper would have no slot to populate).
pub(super) fn sync_video_session(state: &Arc<AppState>, event: &VoiceSessionEvent) {
    let (community_id, channel_id) = {
        let guard = state.voice_engine.lock();
        let Some(handle) = guard.as_ref() else {
            return;
        };
        let Some(community_id) = handle.community_id.clone() else {
            return;
        };
        (community_id, handle.channel_id.clone())
    };
    match event {
        VoiceSessionEvent::UserJoined { peer_pubkey, .. } => {
            if let Err(e) = crate::services::community::video_session::on_peer_joined(
                state,
                &community_id,
                &channel_id,
                peer_pubkey,
            ) {
                tracing::warn!(error = %e, "video_session::on_peer_joined failed");
            }
        }
        VoiceSessionEvent::UserLeft { peer_pubkey } => {
            if let Err(e) = crate::services::community::video_session::on_peer_left(
                state,
                &community_id,
                &channel_id,
                peer_pubkey,
            ) {
                // A late `UserLeft` after `LocalLeft` cleared the slot
                // is a known race — the receive loop may emit one more
                // `UserLeft` after we've torn down. Drop it silently;
                // the slot is already in the post-leave shape.
                tracing::debug!(error = %e, "video_session::on_peer_left ignored");
            }
        }
        _ => {}
    }
}

pub(super) fn map(event: VoiceSessionEvent) -> VoiceEvent {
    match event {
        VoiceSessionEvent::UserJoined {
            peer_pubkey,
            display_name,
        } => VoiceEvent::UserJoined {
            public_key: peer_pubkey,
            display_name,
        },
        VoiceSessionEvent::UserLeft { peer_pubkey } => VoiceEvent::UserLeft {
            public_key: peer_pubkey,
        },
        VoiceSessionEvent::UserSpeaking {
            peer_pubkey,
            speaking,
        } => VoiceEvent::UserSpeaking {
            public_key: peer_pubkey,
            speaking,
        },
        VoiceSessionEvent::UserMuted { peer_pubkey, muted } => VoiceEvent::UserMuted {
            public_key: peer_pubkey,
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
            reason: "voice loop".into(),
            count,
        },
        VoiceSessionEvent::ConnectionQuality { quality } => {
            VoiceEvent::ConnectionQuality { quality }
        }
    }
}

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
    crate::event_dispatch::dispatch(
        app,
        "voice-event",
        &VoiceEvent::LocalJoined {
            channel_id: channel_id.to_string(),
            active_call_type: if community_id.is_some() {
                "community"
            } else {
                "dm"
            }
            .to_string(),
        },
    );
    crate::event_dispatch::dispatch(
        app,
        "voice-event",
        &VoiceEvent::UserJoined {
            public_key: public_key.to_string(),
            display_name: display_name.to_string(),
        },
    );
    crate::event_dispatch::dispatch(
        app,
        "voice-event",
        &VoiceEvent::ConnectionQuality {
            quality: "good".to_string(),
        },
    );
    crate::event_dispatch::dispatch(
        app,
        "voice-event",
        &VoiceEvent::UserSpeaking {
            public_key: public_key.to_string(),
            speaking: false,
        },
    );
}
