//! Phase 14.l — voice session teardown.
//!
//! `shutdown_voice` is the consolidated teardown — shuts down each loop's
//! session scope (cancel, then wait under a deadline) and optionally
//! stops cpal devices + clears the voice packet channels.
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

/// How long each loop scope gets to stop: the session deadline, since a
/// loop may be inside one Veilid call when the stop arrives.
pub const LOOP_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

pub async fn shutdown_voice<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>, opts: &VoiceShutdownOpts) {
    // A FULL shutdown ends the channel session. Read the slot BEFORE the
    // teardown drops the engine handle that names it.
    let ending = if opts.stop_devices {
        let (channel_id, community_id) = deps.active_channel_info();
        community_id.map(|community_id| (community_id, channel_id))
    } else {
        None
    };
    let scopes = deps.take_loop_scopes(*opts);
    // Egress last: the loops enqueue into the links its drivers drain.
    for scope in [scopes.loops, scopes.mcu, scopes.monitor, scopes.egress]
        .into_iter()
        .flatten()
    {
        if let Err(stuck) = scope.shutdown(LOOP_STOP_DEADLINE).await {
            tracing::warn!(%stuck, "voice loops did not stop in time");
        }
    }

    if opts.stop_devices {
        deps.stop_devices_and_clear_engine();
    }

    // W15.5 — clear both voice_packet_tx and the W14.1 staged rx.
    // Without clearing rx_staged, an aborted-before-spawn path leaves
    // an orphaned Receiver that briefly steals packets at the next
    // session start.
    deps.clear_voice_channels();

    if let Some((community_id, channel_id)) = ending {
        deps.end_channel_session(&community_id, &channel_id).await;
    }
}
