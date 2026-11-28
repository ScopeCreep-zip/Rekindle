//! Desktop publication and egress checks for the voice session's media gate.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_types::subscription_events::{
    SubscriptionEvent, VoiceEvent, VoiceMediaReady, VoiceScope,
};
use rekindle_voice::media_ready::MediaReadyInputs;

use crate::state::AppState;
use crate::state_helpers::voice_session_state_for;

pub fn update_media_ready(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    update: impl FnOnce(&mut MediaReadyInputs),
) {
    if let Some(session) = voice_session_state_for(state, community_id, channel_id) {
        session.update_media_ready(update, |status| {
            emit(state, scope(community_id, channel_id), status);
        });
    }
}

/// The hard egress gate — `Err(reason)` until the session converged.
pub fn media_ready_gate(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    let session = voice_session_state_for(state, community_id, channel_id)
        .ok_or_else(|| "not-in-voice".to_string())?;
    session.media_ready_gate()
}

/// Rate-limited counter bump for gate-rejected frames: returns `true`
/// when the caller should emit its warn line (1st and every 30th —
/// one line per ~2 s at 15 fps).
pub fn note_pre_ready_drop(state: &Arc<AppState>) -> u64 {
    state.video_pre_ready_drops.fetch_add(1, Ordering::Relaxed) + 1
}

fn scope(community_id: &str, channel_id: &str) -> VoiceScope {
    VoiceScope::Community {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
    }
}

pub(crate) fn emit(state: &AppState, scope: VoiceScope, status: VoiceMediaReady) {
    tracing::info!(
        target: "rekindle_voice::media_ready",
        community_id = ?scope.community(),
        channel_id = scope.session_key(),
        ready = status.ready,
        reason = %status.reason,
        "media-ready transition"
    );
    let event = SubscriptionEvent::Voice(VoiceEvent::MediaReady {
        scope,
        ready: status.ready,
        reason: status.reason,
    });
    crate::event_dispatch::emit_from_state(
        state,
        crate::event_dispatch::WebviewEvent::Subscription(event),
    );
}
