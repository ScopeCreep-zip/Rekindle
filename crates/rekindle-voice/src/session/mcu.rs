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
use crate::session_deps::VoiceSessionDeps;

pub fn start_mcu_loop<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) -> Result<(), VoiceError> {
    let identity = deps.current_identity()?;
    let our_key_bytes = hex::decode(&identity.public_key).unwrap_or_default();

    let transport = deps
        .current_shared_transport()
        .ok_or_else(|| VoiceError::Session("start_mcu_loop: no active voice engine".into()))?;

    // Pre-stage the MCU's own packet channel and replace the regular
    // voice_packet_tx — the dispatch loop will forward inbound packets
    // here instead of to the receive loop.
    let mcu_packet_rx = deps.pre_stage_mcu_channel();

    let (mcu_shutdown_tx, mcu_shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let mcu_handle = tokio::spawn(mcu_loop::run(mcu_loop::McuParams {
        transport,
        packet_rx: mcu_packet_rx,
        shutdown_rx: mcu_shutdown_rx,
        our_key_bytes,
    }));

    deps.register_mcu_task(mcu_shutdown_tx, mcu_handle);
    tracing::info!("MCU loop started — this peer is the voice host");
    Ok(())
}

pub async fn stop_mcu_loop<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) {
    deps.stop_active_mcu().await;
}
