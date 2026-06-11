//! Runtime glue for the media-ready session gate
//! (`rekindle_voice::media_ready`). Owns the `AppState.media_ready`
//! lock, fans (ready, reason) TRANSITIONS out as
//! `CommunityEvent::VoiceMediaReady`, and exposes the hard egress gate
//! the video send path checks before any frame leaves the process.
//!
//! The decision logic is entirely in the tier crate; this module only
//! locks, emits, and logs (mirrors `video_session.rs`'s split).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_voice::media_ready::MediaReadyInputs;

use crate::channels::CommunityEvent;
use crate::state::AppState;

/// Mutate one (community, channel) slot's bring-up inputs; on a derived
/// (ready, reason) change, log + emit `VoiceMediaReady`. The lock is
/// dropped before the emit (emit path must never hold AppState locks).
pub fn update_media_ready(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    f: impl FnOnce(&mut MediaReadyInputs),
) {
    let transition = {
        let mut tracker = state.media_ready.lock();
        tracker.update(community_id, channel_id, f)
    };
    if let Some(t) = transition {
        tracing::info!(
            target: "rekindle_voice::media_ready",
            community_id,
            channel_id,
            ready = t.ready,
            reason = %t.reason,
            "media-ready transition"
        );
        emit(state, community_id, channel_id, t.ready, t.reason);
    }
}

/// Drop a slot on voice leave/teardown. Emits `ready=false
/// reason="left-channel"` when the slot was previously ready.
pub fn clear_media_ready(state: &Arc<AppState>, community_id: &str, channel_id: &str) {
    let transition = {
        let mut tracker = state.media_ready.lock();
        tracker.clear(community_id, channel_id)
    };
    if let Some(t) = transition {
        tracing::info!(
            target: "rekindle_voice::media_ready",
            community_id,
            channel_id,
            "media-ready cleared on leave"
        );
        emit(state, community_id, channel_id, t.ready, t.reason);
    }
}

/// A MEK landed (any of the rotation/transfer/join paths).
/// `channel_id = None` for a community-level key (the base of the
/// §10.5 channel-media hierarchy — it affects EVERY channel's
/// resolution), `Some(ch)` for a per-channel key (affects only that
/// channel). If a voice session is active on an affected channel,
/// recompute `mek_present` from the resolution. Called from every MEK
/// insert site — cheap no-op when no live voice session is affected.
pub fn on_mek_updated(state: &Arc<AppState>, community_id: &str, channel_id: Option<&str>) {
    let bound_channel = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .filter(|h| h.community_id.as_deref() == Some(community_id))
            .map(|h| h.channel_id.clone())
    };
    let Some(bound_channel) = bound_channel else {
        return;
    };
    if channel_id.is_some_and(|ch| ch != bound_channel) {
        return; // a different channel's key — this session unaffected
    }
    let present =
        crate::state_helpers::channel_media_mek(state, community_id, &bound_channel).is_some();
    update_media_ready(state, community_id, &bound_channel, |i| {
        i.mek_present = present;
    });
}

/// The hard egress gate — `Err(reason)` until the session converged.
pub fn media_ready_gate(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    let (ready, reason) = state.media_ready.lock().state(community_id, channel_id);
    if ready {
        Ok(())
    } else {
        Err(reason)
    }
}

/// Rate-limited counter bump for gate-rejected frames: returns `true`
/// when the caller should emit its warn line (1st and every 30th —
/// one line per ~2 s at 15 fps).
pub fn note_pre_ready_drop(state: &Arc<AppState>) -> u64 {
    state.video_pre_ready_drops.fetch_add(1, Ordering::Relaxed) + 1
}

fn emit(state: &Arc<AppState>, community_id: &str, channel_id: &str, ready: bool, reason: String) {
    let event = CommunityEvent::VoiceMediaReady {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        ready,
        reason,
    };
    crate::event_dispatch::emit_now(state, "community-event", &event);
}
