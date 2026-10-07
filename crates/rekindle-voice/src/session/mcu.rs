//! Phase 14.l — MCU loop lifecycle ports.
//!
//! `start_mcu_loop` is called when this peer is elected voice host
//! (5+ participants per §10.2, or any stage channel per §10.7). The
//! MCU loop drains a dedicated packet receiver, decodes per-sender,
//! mixes per-recipient, and re-encodes.
//!
//! `stop_mcu_loop` is the symmetric teardown — called when another
//! peer becomes the elected host or when we leave the channel.

use std::sync::Arc;

use crate::error::VoiceError;
use crate::mcu_loop;
use crate::media_crypto::{MediaKeys, MediaScope};
use crate::session_deps::{MediaKeySource, VoiceSessionDeps};

pub fn start_mcu_loop(deps: &Arc<dyn VoiceSessionDeps>) -> Result<(), VoiceError> {
    // Self-skip key for the mixer must be the same identity we sign
    // with — the pseudonym for community voice — so the host doesn't
    // mix its own packets back. (MCU only runs for community channels.)
    let community_id = deps.active_community_id();
    let self_voice_id = deps.voice_self_identity(community_id.as_deref());
    let our_key_bytes = hex::decode(&self_voice_id).unwrap_or_default();

    // The MCU opens every inbound frame and seals every mix under the
    // channel's keys, like any participant.
    let (channel_id, _) = deps.active_channel_info();
    let key_source: Arc<dyn MediaKeySource> = deps.clone();
    let keys = Arc::new(MediaKeys::new(
        key_source,
        MediaScope::of_session(community_id.as_deref(), &channel_id),
    ));

    let transport = deps
        .current_shared_transport()
        .ok_or_else(|| VoiceError::Session("start_mcu_loop: no active voice engine".into()))?;

    // Pre-stage the MCU's own packet channel and replace the regular
    // voice_packet_tx — the dispatch loop will forward inbound packets
    // here instead of to the receive loop.
    let mcu_packet_rx = deps.pre_stage_mcu_channel();

    let scope = deps
        .begin_mcu_scope()
        .ok_or_else(|| VoiceError::Session("start_mcu_loop: no active voice engine".into()))?;
    let sends = std::sync::Arc::clone(&scope);
    scope
        .spawn_with_token("voice mcu loop", move |stop| {
            mcu_loop::run(mcu_loop::McuParams {
                transport,
                packet_rx: mcu_packet_rx,
                stop,
                sends,
                our_key_bytes,
                keys,
            })
        })
        .map_err(|e| VoiceError::Session(format!("start_mcu_loop: {e}")))?;
    tracing::info!("MCU loop started — this peer is the voice host");
    Ok(())
}

pub async fn stop_mcu_loop<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) {
    deps.stop_active_mcu().await;
}
