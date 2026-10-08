//! Voice-engine state accessors.
//!
//! Both `voice_adapter` and `voice_signaling_adapter` implement traits
//! that expose mute/deafen, and each had its own copy of the
//! lock-engine-and-set-flag body. The engine handle keeps two pieces of
//! state per toggle — the engine's own setting and an `AtomicBool` the
//! capture path reads without locking — so a copy that updated one and
//! not the other would desync silently. One implementation now.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_types::subscription_events::VoiceScope;

use crate::state::AppState;

/// Mute or unmute the running voice engine, if one is running.
///
/// Sets both the engine's flag and the lock-free `muted_flag` the
/// capture loop polls; they must move together.
/// The scope of the call currently running, if any.
///
/// `VoiceEngineHandle` already carries `channel_id` and an optional
/// `community_id` — the two halves `VoiceScope` needs — so the shape of
/// the active call is read from the engine rather than passed down
/// through every emit site.
///
/// `None` when no call is running. Local-session events (device
/// changes, quality reports) can still fire around a call's edges, so
/// callers must handle that rather than assume a session exists.
pub fn current_voice_scope(state: &Arc<AppState>) -> Option<VoiceScope> {
    let engine = state.voice_engine.lock();
    let handle = engine.as_ref()?;
    Some(match handle.community_id {
        Some(ref community) => VoiceScope::Community {
            community: community.clone(),
            channel: handle.channel_id.clone(),
        },
        // No community means a direct call, and the channel id is the
        // peer we are talking to.
        None => VoiceScope::Dm {
            peer_key: handle.channel_id.clone(),
        },
    })
}

pub fn set_voice_engine_muted(state: &Arc<AppState>, muted: bool) {
    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        handle.engine.set_muted(muted);
        handle.muted_flag.store(muted, Ordering::Relaxed);
    }
}

/// Deafen or undeafen the running voice engine, if one is running.
pub fn set_voice_engine_deafened(state: &Arc<AppState>, deafened: bool) {
    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        handle.engine.set_deafened(deafened);
        handle.deafened_flag.store(deafened, Ordering::Relaxed);
    }
}

/// Whether a voice engine is currently running.
pub fn voice_engine_present(state: &Arc<AppState>) -> bool {
    state.voice_engine.lock().is_some()
}

/// Pseudonym-hex of peers with media-plane evidence (voice packets or
/// receiver reports) within `MEDIA_LIVE_WINDOW_MS`, read from the
/// engine's shared `MediaLiveness` ledger. Empty when no voice engine
/// is active. The ledger's own lock nests strictly inside the engine
/// guard and both are sync, so no `.await` can intervene.
pub fn media_live_peers(state: &Arc<AppState>) -> std::collections::HashSet<String> {
    state
        .voice_engine
        .lock()
        .as_ref()
        .map_or_else(std::collections::HashSet::new, |handle| {
            handle.media_liveness.live_within(
                rekindle_voice::liveness::MEDIA_LIVE_WINDOW_MS,
                rekindle_utils::timestamp_ms(),
            )
        })
}

/// The active voice session's media roster, whatever it is bound to.
pub fn voice_media(
    state: &AppState,
) -> Option<Arc<rekindle_voice::transport::roster::MediaRoster>> {
    state
        .voice_engine
        .lock()
        .as_ref()
        .map(|h| Arc::clone(&h.media))
}

/// Media-plane proof that `peer_hex` is alive: a verified datagram from it
/// (voice, video-plane envelope, padding or transport feedback). The
/// presence reconcile never expires a peer seen here within its window, so
/// a peer that is silent (VAD) but still streaming video, padding or
/// feedback stays on the roster.
pub fn note_media_live(state: &AppState, peer_hex: &str) {
    if let Some(handle) = state.voice_engine.lock().as_ref() {
        handle
            .media_liveness
            .note(peer_hex, rekindle_utils::timestamp_ms());
    }
}

/// The media roster of the voice session bound to
/// `community_id`/`channel_id`: its route queues, without the transport's
/// async lock (plan E4.3.3).
pub fn media_roster_for(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<Arc<rekindle_voice::transport::roster::MediaRoster>> {
    let engine = state.voice_engine.lock();
    let handle = engine.as_ref()?;
    (handle.community_id.as_deref() == Some(community_id) && handle.channel_id == channel_id)
        .then(|| Arc::clone(&handle.media))
}

/// The voice transport when the engine is bound to `community_id` /
/// `channel_id`; `None` when no engine runs or it is on another channel.
/// The `voice_engine` guard drops before this returns, so no sync guard
/// is held across the caller's `.lock().await`.
pub fn voice_transport_for(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>> {
    let engine = state.voice_engine.lock();
    let handle = engine.as_ref()?;
    (handle.community_id.as_deref() == Some(community_id) && handle.channel_id == channel_id)
        .then(|| Arc::clone(&handle.transport))
}
