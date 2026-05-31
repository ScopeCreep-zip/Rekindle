//! Phase 14.l — voice session teardown.
//!
//! `shutdown_voice` is the consolidated teardown — signals each loop's
//! shutdown channel, awaits the join handle, and (optionally) stops
//! cpal devices + clears the voice packet channels.
//!
//! Three named scopes via `VoiceShutdownOpts`:
//! - `FULL`: stop everything + clear engine + clear packet channels.
//! - `LOOPS_ONLY`: stop send/recv/MCU loops only; keep monitor +
//!   engine (used by device hot-swap which runs *on* the monitor).
//! - `KEEP_ENGINE`: stop loops + monitor but keep engine alive (for
//!   restart paths).
//!
//! Architecture compliance: per VeilidChat §3 ("no explicit per-route
//! teardown; lazy closing") we do NOT explicitly close Veilid routes
//! here — they're released when the engine drops or the routing
//! context is GC'd. We only stop cpal devices + signal our own task
//! shutdowns.

use std::sync::Arc;

use crate::session_deps::{VoiceSessionDeps, VoiceShutdownOpts};

pub async fn shutdown_voice<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>, opts: &VoiceShutdownOpts) {
    let handles = deps.take_shutdown_handles(*opts);

    // Signal every taken shutdown channel.
    if let Some(tx) = handles.send_loop_shutdown {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = handles.recv_loop_shutdown {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = handles.monitor_shutdown {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = handles.mcu_shutdown {
        let _ = tx.send(()).await;
    }

    // Await each loop's exit.
    if let Some(h) = handles.send_loop_handle {
        let _ = h.await;
    }
    if let Some(h) = handles.recv_loop_handle {
        let _ = h.await;
    }
    if let Some(h) = handles.monitor_handle {
        let _ = h.await;
    }
    if let Some(h) = handles.mcu_handle {
        let _ = h.await;
    }

    if opts.stop_devices {
        deps.stop_devices_and_clear_engine();
    }

    // W15.5 — clear both voice_packet_tx and the W14.1 staged rx.
    // Without clearing rx_staged, an aborted-before-spawn path leaves
    // an orphaned Receiver that briefly steals packets at the next
    // session start.
    deps.clear_voice_channels();
}
