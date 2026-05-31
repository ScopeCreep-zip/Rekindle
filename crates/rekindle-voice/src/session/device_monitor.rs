//! Phase 14.l — audio device monitor loop.
//!
//! Watches for cpal device disappearance via two signals:
//! 1. cpal stream-error callbacks (device disconnected mid-stream)
//! 2. periodic (5s) device-enumeration polling (default-routing
//!    changes that don't fire callbacks)
//!
//! On either signal: hot-swap to the OS defaults by calling
//! `shutdown_voice(LOOPS_ONLY)` + clearing the engine's device
//! config + `restart_loops`. After the swap fires, this loop instance
//! exits because `restart_loops` spawns a fresh monitor.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::session_deps::{VoiceSessionDeps, VoiceShutdownOpts};

pub struct DeviceMonitorParams<D: VoiceSessionDeps + ?Sized> {
    pub device_error_rx: mpsc::Receiver<String>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub deps: Arc<D>,
}

pub async fn run<D: VoiceSessionDeps + ?Sized>(mut params: DeviceMonitorParams<D>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!("device monitor loop started");

    loop {
        tokio::select! {
            biased;

            _ = params.shutdown_rx.recv() => {
                tracing::info!("device monitor loop: shutdown signal received");
                break;
            }

            Some(error_msg) = params.device_error_rx.recv() => {
                tracing::warn!(error = %error_msg, "device monitor: cpal stream error detected");
                let device_type = classify_device_error(&error_msg);
                if let Err(e) = handle_device_swap(&params.deps, device_type, "disconnected").await {
                    tracing::error!(error = %e, "device monitor: hot-swap failed after error");
                }
                // restart_loops spawned a fresh monitor — this instance must exit.
                break;
            }

            _ = tick.tick() => {
                if let Some(device_type) = check_device_availability(&params.deps) {
                    if let Err(e) = handle_device_swap(&params.deps, device_type, "disconnected").await {
                        tracing::error!(error = %e, "device monitor: {device_type} hot-swap failed");
                    }
                    break;
                }
            }
        }
    }

    tracing::info!("device monitor loop exited");
}

fn classify_device_error(error_msg: &str) -> &'static str {
    if error_msg.starts_with("input:") {
        "input"
    } else {
        "output"
    }
}

/// If a selected device disappeared, return which kind. `None` means
/// devices are still present (or both are default — OS handles those).
fn check_device_availability<D: VoiceSessionDeps + ?Sized>(deps: &Arc<D>) -> Option<&'static str> {
    let (input_device, output_device) = deps.voice_engine_device_config();

    // Both defaults → OS handles re-routing automatically.
    if input_device.is_none() && output_device.is_none() {
        return None;
    }
    // No engine? Stop monitoring.
    if !deps.voice_engine_present() {
        return None;
    }

    let devices = crate::capture::enumerate_audio_devices();
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

    if let Some(ref name) = input_device {
        if !input_names.contains(&name.as_str()) {
            tracing::warn!(device = %name, "device monitor: selected input device disappeared");
            return Some("input");
        }
    }
    if let Some(ref name) = output_device {
        if !output_names.contains(&name.as_str()) {
            tracing::warn!(device = %name, "device monitor: selected output device disappeared");
            return Some("output");
        }
    }

    None
}

/// Hot-swap to system defaults. Caller (the monitor loop) MUST exit
/// after this — `restart_loops` spawns a fresh monitor to replace it.
async fn handle_device_swap<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    device_type: &'static str,
    reason: &str,
) -> Result<(), crate::error::VoiceError> {
    if !deps.voice_engine_present() {
        return Ok(());
    }

    // LOOPS_ONLY: don't stop the monitor — we ARE the monitor; awaiting
    // our own JoinHandle would deadlock.
    crate::session::shutdown::shutdown_voice(deps, &VoiceShutdownOpts::LOOPS_ONLY).await;

    // Stop cpal + reset to OS defaults, then restart.
    deps.stop_audio_devices();
    deps.set_voice_engine_devices(None, None);
    crate::session::restart::restart_loops(deps).await?;

    deps.emit_device_changed(
        device_type.to_string(),
        "default".to_string(),
        reason.to_string(),
    );
    deps.emit_system_alert(
        "Audio Device Disconnected".to_string(),
        format!("Your {device_type} device was disconnected. Switched to default device."),
    );

    tracing::info!(
        device_type,
        reason,
        "device monitor: hot-swapped to default device"
    );
    Ok(())
}
