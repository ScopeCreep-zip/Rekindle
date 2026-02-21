//! Device availability monitoring and hot-swap.
//!
//! Monitors audio device availability via error callbacks and periodic polling.
//! On device error (cpal callback) or when a selected device disappears from
//! the enumerated list, performs a hot-swap to the default device.

use std::time::Duration;

use tauri::Emitter;
use tokio::sync::mpsc;

use crate::channels::{NotificationEvent, VoiceEvent};
use crate::state::SharedState;

pub(crate) struct DeviceMonitorParams {
    pub device_error_rx: mpsc::Receiver<String>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub app: tauri::AppHandle,
    pub state: SharedState,
}

struct DeviceMonitor {
    device_error_rx: mpsc::Receiver<String>,
    shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    state: SharedState,
}

/// Entry point: build monitor state and run the loop.
pub(crate) async fn run(params: DeviceMonitorParams) {
    let monitor = DeviceMonitor {
        device_error_rx: params.device_error_rx,
        shutdown_rx: params.shutdown_rx,
        app: params.app,
        state: params.state,
    };
    monitor.run_loop().await;
}

impl DeviceMonitor {
    async fn run_loop(mut self) {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        tracing::info!("device monitor loop started");

        loop {
            tokio::select! {
                biased;

                _ = self.shutdown_rx.recv() => {
                    tracing::info!("device monitor loop: shutdown signal received");
                    break;
                }

                Some(error_msg) = self.device_error_rx.recv() => {
                    tracing::warn!(error = %error_msg, "device monitor: cpal stream error detected");

                    let device_type = classify_device_error(&error_msg);

                    // Hot-swap to default devices. restart_loops spawns a new
                    // monitor, so this instance must exit after the swap.
                    if let Err(e) = handle_device_swap(&self.app, &self.state, device_type, "disconnected").await {
                        tracing::error!(error = %e, "device monitor: hot-swap failed after error");
                    }
                    break;
                }

                _ = tick.tick() => {
                    if let Some(device_type) = self.check_device_availability() {
                        if let Err(e) = handle_device_swap(&self.app, &self.state, device_type, "disconnected").await {
                            tracing::error!(error = %e, "device monitor: {device_type} hot-swap failed");
                        }
                        // restart_loops spawned a new monitor — this one must exit
                        break;
                    }
                }
            }
        }

        tracing::info!("device monitor loop exited");
    }

    fn check_device_availability(&self) -> Option<&'static str> {
        // Read currently selected device names from the engine
        let (input_device, output_device) = {
            let ve = self.state.voice_engine.lock();
            match ve.as_ref() {
                Some(handle) => {
                    let cfg = handle.engine.config();
                    (cfg.input_device.clone(), cfg.output_device.clone())
                }
                None => return None, // Engine gone — stop monitoring
            }
        };

        // If using defaults (None), skip check — OS handles default routing
        if input_device.is_none() && output_device.is_none() {
            return None;
        }

        // Enumerate current devices
        let devices = match rekindle_voice::capture::enumerate_audio_devices() {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!(error = %e, "device monitor: enumeration failed");
                return None;
            }
        };

        let input_names: Vec<&str> = devices
            .input_devices
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let output_names: Vec<&str> = devices
            .output_devices
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();

        // Check if selected input device disappeared
        if let Some(ref name) = input_device {
            if !input_names.contains(&name.as_str()) {
                tracing::warn!(device = %name, "device monitor: selected input device disappeared");
                return Some("input");
            }
        }

        // Check if selected output device disappeared
        if let Some(ref name) = output_device {
            if !output_names.contains(&name.as_str()) {
                tracing::warn!(device = %name, "device monitor: selected output device disappeared");
                return Some("output");
            }
        }

        None
    }
}

fn classify_device_error(error_msg: &str) -> &'static str {
    if error_msg.starts_with("input:") {
        "input"
    } else {
        "output"
    }
}

/// Perform a device hot-swap to defaults when a device disappears.
///
/// Only shuts down send/recv loops (NOT the device monitor — this is called
/// FROM the monitor loop, so awaiting its own handle would deadlock).
/// After this returns, the caller (`DeviceMonitor`) MUST break out of its
/// loop — `restart_loops` spawns a fresh monitor to replace it.
pub(crate) async fn handle_device_swap(
    app: &tauri::AppHandle,
    state: &SharedState,
    device_type: &str,
    reason: &str,
) -> Result<(), String> {
    // Check voice engine is still active
    {
        let ve = state.voice_engine.lock();
        if ve.is_none() {
            return Ok(());
        }
    }

    // Shut down send/recv loops only — NOT the monitor (we ARE the monitor)
    super::shutdown::shutdown_voice(state, &super::shutdown::VoiceShutdownOpts::LOOPS_ONLY).await;

    // Stop capture/playback and switch to default devices
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
            handle.engine.set_devices(None, None);
        }
    }

    // Restart with default devices — this spawns new send/recv loops AND a new monitor
    super::session::restart_loops(state, app)?;

    // Emit DeviceChanged event
    let event = VoiceEvent::DeviceChanged {
        device_type: device_type.to_string(),
        device_name: "default".to_string(),
        reason: reason.to_string(),
    };
    let _ = app.emit("voice-event", &event);

    // Emit notification
    let notification = NotificationEvent::SystemAlert {
        title: "Audio Device Disconnected".to_string(),
        body: format!("Your {device_type} device was disconnected. Switched to default device."),
    };
    let _ = app.emit("notification-event", &notification);

    tracing::info!(
        device_type,
        reason,
        "device monitor: hot-swapped to default device"
    );

    Ok(())
}
