//! Phase 14.r split — pure `VoiceSessionEvent → VoiceEvent` mapping +
//! the small `emit_local_joined` fan-out.
//!
//! Extracted from the trait impl so deps_impl stays under the 500
//! LoC cap. No state access — pure value mapping; the fan-out takes
//! `AppHandle` by reference and uses it only for emit calls.

use std::sync::Arc;

use rekindle_voice::VoiceSessionEvent;

use rekindle_types::subscription_events::{
    LinkMeasurement, SubscriptionEvent, VoiceEvent, VoiceScope,
};

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
    pub rx_mek_drops: u64,
    /// The send half's end-to-end measurement, from receiver reports.
    /// `None` until a peer reports.
    pub link: Option<LinkMeasurement>,
}

impl Default for VoiceQualityCache {
    fn default() -> Self {
        Self {
            quality: "good".to_string(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_mek_drops: 0,
            link: None,
        }
    }
}

/// Project the crate's measurement types onto the flat Tier-1 event
/// payload. The one place the two representations meet — see
/// [`LinkMeasurement`]'s note on why they are separate.
fn project_link(stats: &rekindle_voice::session_deps::SendLinkStats) -> LinkMeasurement {
    LinkMeasurement {
        loss_q8: stats.metrics.loss_rate_q8,
        discard_q8: stats.metrics.discard_rate_q8,
        jitter_ms: stats.metrics.jitter_ms,
        rtt_ms: stats.rtt_ms,
        r_factor: stats.score.r_factor,
        mos_lq: stats.score.mos_lq,
        mos_cq: stats.score.mos_cq,
        bitrate_bps: stats.bitrate_bps,
    }
}

/// Phase 5 — fold a quality/receive-stats event into the cache and
/// build the merged UI event. Returns `None` for every other variant.
pub(super) fn merge_quality_event(
    state: &Arc<AppState>,
    event: &rekindle_voice::VoiceSessionEvent,
    scope: &VoiceScope,
) -> Option<VoiceEvent> {
    use rekindle_voice::VoiceSessionEvent as E;
    let mut cache = state.voice_quality_cache.lock();
    match event {
        E::ConnectionQuality { quality, link } => {
            cache.quality.clone_from(quality);
            // Only overwrite when this window carried a measurement; a
            // window with no report should not erase the last known
            // numbers, or the UI blinks empty between reports.
            if let Some(stats) = link {
                cache.link = Some(project_link(stats));
                // Flag voice-route pressure so the video bitrate
                // controller yields egress. Voice and video share the
                // peer's media route; without this, video's AIMD climbs
                // to its ceiling on its own low loss and starves audio
                // (observed: voice RTT spiking to ~16 s while video sat
                // at 600 kbps). Keyed off the E-model verdict the send
                // loop already computed — the same `quality` string the
                // UI shows — rather than a second hand-tuned threshold.
                if quality == "poor" || quality == "lost" {
                    state.voice_route_pressure_ms.store(
                        rekindle_utils::timestamp_ms(),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
        }
        E::ReceiveStats {
            rx_overflow_drops,
            rx_late_drops,
            rx_mek_drops,
        } => {
            cache.rx_overflow_drops = *rx_overflow_drops;
            cache.rx_late_drops = *rx_late_drops;
            cache.rx_mek_drops = *rx_mek_drops;
        }
        _ => return None,
    }
    Some(VoiceEvent::ConnectionQuality {
        scope: scope.clone(),
        quality: cache.quality.clone(),
        rx_overflow_drops: cache.rx_overflow_drops,
        rx_late_drops: cache.rx_late_drops,
        rx_mek_drops: cache.rx_mek_drops,
        ingress_drops: state
            .voice_ingress_drops_total
            .load(std::sync::atomic::Ordering::Relaxed),
        link: cache.link,
    })
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

    for event in [
        VoiceEvent::LocalJoined {
            scope: scope.clone(),
        },
        VoiceEvent::Joined {
            scope: scope.clone(),
            pseudonym: public_key.to_string(),
            display_name: Some(display_name.to_string()),
            // This is us joining our own call; nobody needs a route to
            // reach us from this event.
            route_blob: None,
        },
        VoiceEvent::ConnectionQuality {
            scope: scope.clone(),
            quality: "good".to_string(),
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_mek_drops: 0,
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
