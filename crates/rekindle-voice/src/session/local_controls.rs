//! Phase 14.o — local-user voice control entry points.
//!
//! Tauri commands (`commands::voice::*`) wrap these. Each entry point
//! flips one piece of voice engine state via `VoiceSessionDeps` and
//! (where relevant) emits the matching `VoiceSessionEvent` for UI
//! reflection.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::error::VoiceError;
use crate::session_deps::{VoiceSessionDeps, VoiceSessionEvent, VoiceShutdownOpts};

/// Local user clicked Mute. Flips the engine's muted flag + emits
/// `UserMuted` so other frontends mirror the state. Infallible —
/// missing engine is a no-op.
pub fn set_local_mute<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>, muted: bool) {
    deps.set_voice_engine_muted(muted);
    let public_key = deps.owner_key().unwrap_or_default();
    deps.emit_voice_event(VoiceSessionEvent::UserMuted {
        peer_pubkey: public_key,
        muted,
    });
}

/// Local user clicked Deafen. No emit — the deafen state is local-only
/// (peers don't need to know if we can hear them).
pub fn set_local_deafen<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>, deafened: bool) {
    deps.set_voice_engine_deafened(deafened);
}

/// User clicked Join on a voice channel. Brings up the voice session
/// then (for community channels) broadcasts VoiceJoin so other
/// participants add us as a peer + records analytics. Wraps
/// `start_session` plus the community-only gossip + DB side effects.
pub async fn join_voice_channel<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    channel_id: &str,
    community_id: Option<&str>,
) -> Result<(), VoiceError> {
    crate::session::start_session(deps, channel_id, community_id).await?;

    if let Some(cid) = community_id {
        deps.log_voice_membership(cid, channel_id, true);
        let route_blob = deps.our_route_blob();
        let envelope = CommunityEnvelope::Control(ControlPayload::VoiceJoin {
            channel_id: channel_id.to_string(),
            route_blob,
        });
        deps.send_community_envelope(cid, &envelope);
    }
    Ok(())
}

/// User clicked Leave. Reads the current channel/community off the
/// engine, broadcasts VoiceLeave (community channels only),
/// tears down the voice session, emits UserLeft.
pub async fn leave_voice<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) -> Result<(), VoiceError> {
    let public_key = deps.owner_key().unwrap_or_default();
    let (channel_id, community_id) = deps.active_channel_info();

    if let Some(ref cid) = community_id {
        deps.log_voice_membership(cid, &channel_id, false);
        let envelope = CommunityEnvelope::Control(ControlPayload::VoiceLeave {
            channel_id: channel_id.clone(),
        });
        deps.send_community_envelope(cid, &envelope);
    }

    crate::session::shutdown_voice(deps, &VoiceShutdownOpts::FULL).await;

    deps.emit_voice_event(VoiceSessionEvent::UserLeft {
        peer_pubkey: public_key,
    });
    Ok(())
}

/// Hot-swap audio devices mid-call. No-op when no voice session is
/// active (settings persist in the Tauri store separately, handled
/// by the command facade). When active: shutdown loops (KEEP_ENGINE),
/// stop devices, swap the engine's device config, restart loops.
/// ~100 ms audio interruption.
pub async fn change_audio_devices<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    input: Option<String>,
    output: Option<String>,
) -> Result<(), VoiceError> {
    if !deps.voice_engine_present() {
        return Ok(()); // No active call — settings take effect next join.
    }
    tracing::info!(?input, ?output, "hot-swapping audio devices mid-call");
    crate::session::shutdown_voice(deps, &VoiceShutdownOpts::KEEP_ENGINE).await;
    deps.stop_audio_devices();
    deps.set_voice_engine_devices(input, output);
    crate::session::restart_loops(deps).await?;
    Ok(())
}

/// Switch voice mode between mesh and MCU. Stops any existing MCU
/// loop, flips the transport mode, and (if we're the new host)
/// starts the MCU mixing loop on the shared transport.
pub async fn set_voice_mode<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    mode: &str,
    host_pseudonym: Option<String>,
) -> Result<(), VoiceError> {
    crate::session::stop_mcu_loop(deps).await;

    if let Some(transport) = deps.current_shared_transport() {
        let mode_enum = if mode == "mcu" {
            crate::VoiceMode::Mcu {
                host_pseudonym: host_pseudonym.clone().unwrap_or_default(),
            }
        } else {
            crate::VoiceMode::Mesh
        };
        transport.lock().await.set_mode(mode_enum);
    }

    if mode == "mcu" {
        if let Some(ref host) = host_pseudonym {
            let my_pseudonym = deps.owner_key().unwrap_or_default();
            if *host == my_pseudonym {
                crate::session::start_mcu_loop(deps)?;
            }
        }
    }

    Ok(())
}
