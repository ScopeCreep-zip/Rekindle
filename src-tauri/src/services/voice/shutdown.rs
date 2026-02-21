//! Voice shutdown with configurable scope.
//!
//! Moved from `commands/voice.rs` to allow both commands and services
//! (e.g. `veilid_service::logout_cleanup`) to call `shutdown_voice` directly.

use crate::state::AppState;

/// What to shut down when tearing down voice state.
pub(crate) struct VoiceShutdownOpts {
    /// Stop send and receive loops.
    pub stop_loops: bool,
    /// Stop the device monitor loop.
    pub stop_monitor: bool,
    /// Stop audio capture/playback devices and clear the engine handle.
    pub stop_devices: bool,
}

impl VoiceShutdownOpts {
    /// Shut down everything: loops, monitor, and devices.
    pub const FULL: Self = Self {
        stop_loops: true,
        stop_monitor: true,
        stop_devices: true,
    };
    /// Shut down only send/receive loops; keep monitor and engine alive.
    pub const LOOPS_ONLY: Self = Self {
        stop_loops: true,
        stop_monitor: false,
        stop_devices: false,
    };
    /// Shut down loops and monitor but keep the engine alive (for device hot-swap).
    pub const KEEP_ENGINE: Self = Self {
        stop_loops: true,
        stop_monitor: true,
        stop_devices: false,
    };
}

/// Consolidated voice shutdown — signal loops, await completion, optionally stop devices.
///
/// Takes `&AppState` (not `&SharedState`) because it only accesses direct fields
/// (`voice_engine`, `voice_packet_tx`). Callers with `&Arc<AppState>` auto-deref.
pub(crate) async fn shutdown_voice(state: &AppState, opts: &VoiceShutdownOpts) {
    // Extract shutdown senders and join handles outside the lock
    let (send_tx, send_h, recv_tx, recv_h, monitor_tx, monitor_h) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            let loops = if opts.stop_loops {
                (
                    handle.send_loop_shutdown.take(),
                    handle.send_loop_handle.take(),
                    handle.recv_loop_shutdown.take(),
                    handle.recv_loop_handle.take(),
                )
            } else {
                (None, None, None, None)
            };
            let monitor = if opts.stop_monitor {
                (
                    handle.device_monitor_shutdown.take(),
                    handle.device_monitor_handle.take(),
                )
            } else {
                (None, None)
            };
            (loops.0, loops.1, loops.2, loops.3, monitor.0, monitor.1)
        } else {
            (None, None, None, None, None, None)
        }
    };

    // Signal loops to shut down
    if let Some(tx) = send_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = recv_tx {
        let _ = tx.send(()).await;
    }
    if let Some(tx) = monitor_tx {
        let _ = tx.send(()).await;
    }

    // Await loop handles
    if let Some(h) = send_h {
        let _ = h.await;
    }
    if let Some(h) = recv_h {
        let _ = h.await;
    }
    if let Some(h) = monitor_h {
        let _ = h.await;
    }

    // Stop audio devices and clear engine (when requested)
    if opts.stop_devices {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
        *ve = None;
    }

    // Clear voice packet channel
    *state.voice_packet_tx.write() = None;
}
