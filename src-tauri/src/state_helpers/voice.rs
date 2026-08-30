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

use crate::state::AppState;

/// Mute or unmute the running voice engine, if one is running.
///
/// Sets both the engine's flag and the lock-free `muted_flag` the
/// capture loop polls; they must move together.
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
