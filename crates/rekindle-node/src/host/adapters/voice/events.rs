//! Local voice events and the current quality measurements.

use rekindle_calls::signaling::deps::friend_shown_name;
use rekindle_types::key_format::pseudonym_hex;
use rekindle_voice::VoiceSessionEvent;

use rekindle_types::subscription_events::{
    LinkMeasurement, SubscriptionEvent, VoiceEvent, VoiceQuality, VoiceScope,
};

use super::deps::HostVoiceDeps;

/// Last-known halves of the merged `ConnectionQuality` UI event. The
/// send loop reports `quality` and the receive loop reports jitter
/// drops on independent 5 s cadences; each emission carries the full
/// picture from this cache.
#[derive(Debug, Clone)]
pub struct VoiceQualityCache {
    pub quality: VoiceQuality,
    pub rx_overflow_drops: u64,
    pub rx_late_drops: u64,
    pub rx_key_drops: u64,
    /// The send half's end-to-end measurement, from receiver reports.
    /// `None` until a peer reports.
    pub link: Option<LinkMeasurement>,
}

impl Default for VoiceQualityCache {
    fn default() -> Self {
        Self {
            quality: VoiceQuality::Good,
            rx_overflow_drops: 0,
            rx_late_drops: 0,
            rx_key_drops: 0,
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
        one_way_p50_ms: stats.route.map(|r| r.one_way_p50_ms),
        one_way_p95_ms: stats.route.map(|r| r.one_way_p95_ms),
        mouth_to_ear_ms: stats.route.map(|r| r.mouth_to_ear_ms),
        outside_g114: stats.route.is_some_and(|r| r.outside_g114),
    }
}

/// Phase 5 — fold a quality/receive-stats event into the cache and
/// build the merged UI event. Returns `None` for every other variant.
pub(super) fn merge_quality_event(
    deps: &HostVoiceDeps,
    event: &rekindle_voice::VoiceSessionEvent,
    scope: &VoiceScope,
) -> Option<VoiceEvent> {
    use rekindle_voice::VoiceSessionEvent as E;
    let mut cache = deps.voice().quality.lock();
    match event {
        E::ConnectionQuality { quality, link } => {
            cache.quality = *quality;
            // Only overwrite when this window carried a measurement; a
            // window with no report should not erase the last known
            // numbers, or the UI blinks empty between reports.
            if let Some(stats) = link {
                cache.link = Some(project_link(stats));
            }
        }
        E::ReceiveStats {
            rx_overflow_drops,
            rx_late_drops,
            rx_key_drops,
        } => {
            cache.rx_overflow_drops = *rx_overflow_drops;
            cache.rx_late_drops = *rx_late_drops;
            cache.rx_key_drops = *rx_key_drops;
        }
        _ => return None,
    }
    Some(VoiceEvent::ConnectionQuality {
        scope: scope.clone(),
        quality: cache.quality,
        notice: cache.quality.notice().into(),
        rx_overflow_drops: cache.rx_overflow_drops,
        rx_late_drops: cache.rx_late_drops,
        rx_key_drops: cache.rx_key_drops,
        ingress_drops: deps.voice().voice_drops(),
        link: cache.link,
    })
}

pub(super) fn map(event: VoiceSessionEvent, scope: &VoiceScope) -> VoiceEvent {
    match event {
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
        VoiceSessionEvent::UserJoined { .. }
        | VoiceSessionEvent::ConnectionQuality { .. }
        | VoiceSessionEvent::ReceiveStats { .. } => {
            unreachable!("joined and quality events are handled by emit_voice_event")
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
pub(super) fn emit_local_joined(
    deps: &HostVoiceDeps,
    session: &rekindle_voice::session_state::VoiceSessionState,
    channel_id: &str,
    community_id: Option<&str>,
    public_key: &str,
    display_name: &str,
) {
    // The call's shape, built here rather than read from the engine:
    // this fires as the session comes up, so the handle may not be
    // installed yet. `community_id` is exactly what decided the old
    // `active_call_type` string, so the two carry the same fact.
    let scope = scope_of(channel_id, community_id);
    let name = match community_id {
        Some(community) => {
            if let Err(error) = pseudonym_hex(public_key) {
                tracing::warn!(%error, "invalid local community voice participant");
                return;
            }
            deps.governance().member_shown_name(community, public_key)
        }
        None => match friend_shown_name(public_key, Some(display_name.to_owned())) {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(%error, "invalid local DM voice participant");
                return;
            }
        },
    };
    session.emit_local_joined(scope.clone(), |event| {
        deps.ctx()
            .journal
            .publish_state(SESSION_STATE, SubscriptionEvent::Voice(event));
    });
    deps.ctx().journal.publish_state(
        SELF_STATE,
        SubscriptionEvent::Voice(VoiceEvent::Joined {
            scope: scope.clone(),
            pseudonym: public_key.to_owned(),
            display_name: name,
            route_blob: None,
        }),
    );
    for event in [
        VoiceEvent::ConnectionQuality {
            scope: scope.clone(),
            quality: VoiceQuality::Good,
            notice: VoiceQuality::Good.notice().into(),
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
        deps.publish(event);
    }
}

pub(super) fn scope_of(channel: &str, community: Option<&str>) -> VoiceScope {
    match community {
        Some(community) => VoiceScope::Community {
            community: community.to_owned(),
            channel: channel.to_owned(),
        },
        None => VoiceScope::Dm {
            peer_key: channel.to_owned(),
        },
    }
}

pub(super) fn emit(deps: &HostVoiceDeps, scope: VoiceScope, event: VoiceSessionEvent) {
    if let Some(merged) = merge_quality_event(deps, &event, &scope) {
        deps.publish(merged);
        return;
    }
    let event = match event {
        VoiceSessionEvent::UserJoined { peer_pubkey } => {
            let display_name = match &scope {
                VoiceScope::Community { community, .. } => {
                    if let Err(error) = pseudonym_hex(&peer_pubkey) {
                        tracing::warn!(%error, "invalid community voice participant");
                        return;
                    }
                    deps.governance().member_shown_name(community, &peer_pubkey)
                }
                VoiceScope::Dm { .. } => {
                    let name = deps
                        .ctx()
                        .session
                        .read()
                        .as_ref()
                        .and_then(|session| session.friends.get(&peer_pubkey))
                        .map(|friend| friend.display_name.clone());
                    match friend_shown_name(&peer_pubkey, name) {
                        Ok(name) => name,
                        Err(error) => {
                            tracing::warn!(%error, "invalid direct voice participant");
                            return;
                        }
                    }
                }
            };
            VoiceEvent::Joined {
                scope,
                pseudonym: peer_pubkey,
                display_name,
                route_blob: None,
            }
        }
        other => map(other, &scope),
    };
    deps.publish(event);
}
