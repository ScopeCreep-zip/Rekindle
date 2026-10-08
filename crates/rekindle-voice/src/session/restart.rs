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
pub async fn restart_loops<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) -> Result<(), VoiceError> {
    deps.restart_audio_devices()?;

    let shared_transport = deps
        .current_shared_transport()
        .ok_or_else(|| VoiceError::Session("restart_loops: no active voice engine".into()))?;
    let (muted_flag, deafened_flag) = deps.current_voice_flags()?;

    let community_id = deps.active_community_id();
    let member_names = deps.load_member_names(community_id.as_deref()).await;
    // The key we are on the voice wire, as `start_voice` spawns with: the
    // community pseudonym in a channel, the account key in a 1:1 call.
    // Receiver reports and transport feedback name it as their signer and
    // are signed with its key; the account key here made every report after
    // a device change fail verification at the peer.
    let self_voice_id = deps.voice_self_identity(community_id.as_deref());

    deps.spawn_voice_loops(
        &self_voice_id,
        shared_transport,
        muted_flag,
        deafened_flag,
        member_names,
    )?;

    Ok(())
}
