//! Phase 14.l — `restart_loops` orchestrator port.
//!
//! Hot-swap path used after `shutdown_voice(KEEP_ENGINE)` to bring
//! the loops back up on the existing engine (e.g. when the user
//! switched audio devices in settings). Crucially, this path
//! **reuses the existing shared transport** — recreating it would
//! drop all peers already added via VoiceJoin gossip.

use std::sync::Arc;

use crate::error::VoiceError;
use crate::session_deps::VoiceSessionDeps;

/// Re-open cpal capture+playback on the existing engine, then
/// respawn the three voice loops against the unchanged shared
/// transport.
pub async fn restart_loops<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
) -> Result<(), VoiceError> {
    let identity = deps.current_identity()?;

    deps.restart_audio_devices()?;

    let shared_transport = deps.current_shared_transport().ok_or_else(|| {
        VoiceError::Session("restart_loops: no active voice engine".into())
    })?;
    let (muted_flag, deafened_flag) = deps.current_voice_flags()?;

    let community_id = deps.active_community_id();
    let member_names = deps.load_member_names(community_id.as_deref()).await;

    deps.spawn_voice_loops(
        &identity.public_key,
        shared_transport,
        muted_flag,
        deafened_flag,
        member_names,
    )?;

    Ok(())
}
