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

/// Last-known halves of the merged `ConnectionQuality` UI event. The
/// send loop reports `quality` and the receive loop reports jitter
/// drops on independent 5 s cadences; each emission carries the full
/// picture from this cache.
#[derive(Debug, Clone)]
pub struct VoiceQualityCache {
    pub quality: String,
    pub rx_overflow_drops: u64,
    pub rx_late_drops: u64,
}

impl Default for VoiceQualityCache {
    fn default() -> Self {
        Self {
            quality: "good".to_string(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
        }
    }
}

/// Phase 5 — fold a quality/receive-stats event into the cache and
/// build the merged UI event. Returns `None` for every other variant.
pub(super) fn merge_quality_event(
    state: &Arc<AppState>,
    event: &rekindle_voice::VoiceSessionEvent,
) -> Option<VoiceEvent> {
    use rekindle_voice::VoiceSessionEvent as E;
    let mut cache = state.voice_quality_cache.lock();
    match event {
        E::ConnectionQuality { quality } => {
            cache.quality.clone_from(quality);
        }
        E::ReceiveStats {
            rx_overflow_drops,
            rx_late_drops,
        } => {
            cache.rx_overflow_drops = *rx_overflow_drops;
            cache.rx_late_drops = *rx_late_drops;
        }
        _ => return None,
    }
    Some(VoiceEvent::ConnectionQuality {
        quality: cache.quality.clone(),
        rx_overflow_drops: cache.rx_overflow_drops,
        rx_late_drops: cache.rx_late_drops,
        ingress_drops: state
            .voice_ingress_drops_total
            .load(std::sync::atomic::Ordering::Relaxed),
    })
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
        VoiceSessionEvent::ConnectionQuality { .. } | VoiceSessionEvent::ReceiveStats { .. } => {
            unreachable!(
                "quality/receive-stats are merged + emitted in emit_voice_event before map()"
            )
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
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            ingress_drops: 0,
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
