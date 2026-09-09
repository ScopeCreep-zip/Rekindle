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
