//! Audio device monitor loop (plans 14.l, C7.24).
//!
//! Keeps the open capture and playback devices on the ones a call should
//! use: the saved device when it is connected, otherwise the system
//! default ([`crate::device::select_device`]). It re-checks on two signals:
//! 1. cpal stream-error callbacks: the device went away, or the system
//!    default changed under the stream (cpal 0.18 reports both on PipeWire,
//!    and default-output changes on CoreAudio);
//! 2. a periodic (5 s) check, because no host reports a device being added:
//!    a saved device plugged back in reaches no open stream on its own.
//!    The check reads the host's device list. On Linux that is the
//!    PipeWire (or PulseAudio) server's registry, which opens nothing;
//!    cpal's ALSA enumeration, by contrast, opens every PCM in both
//!    directions to list it, which is why the sound-server hosts are on.
//!
//! When the target differs from what is open, it reopens on the target —
//! switching to the default when the saved device goes, back to the saved
//! device when it returns (Jitsi Meet `getNewAudioInputDevice`), and to a
//! new system default — and says so. The saved choice is never cleared.
//! After a swap this loop instance exits: `restart_loops` spawns a fresh
//! monitor.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::device::{select_device, DeviceDirection, SelectedDevice};
use crate::session_deps::{AudioPrefs, VoiceSessionDeps, VoiceShutdownOpts};
use crate::VoiceError;

/// The concrete devices a call should have open right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioTargets {
    pub input: SelectedDevice,
    pub output: SelectedDevice,
}

impl AudioTargets {
    /// Resolve the saved choices in `prefs` against the connected devices.
    pub(crate) fn select(prefs: &AudioPrefs) -> Result<Self, VoiceError> {
        let host = cpal::default_host();
        Ok(Self {
            input: select_device(
                &host,
                prefs.input_device.as_deref(),
                &DeviceDirection::Input,
            )?,
            output: select_device(
                &host,
                prefs.output_device.as_deref(),
                &DeviceDirection::Output,
            )?,
        })
    }

    /// Whether these are the devices already open.
    fn match_open(&self, open: &(Option<String>, Option<String>)) -> bool {
        open.0.as_deref() == Some(self.input.id.as_str())
            && open.1.as_deref() == Some(self.output.id.as_str())
    }

    /// Tell the user about each stand-in for a saved device that is not
    /// connected.
    pub(crate) fn announce_missing<D: VoiceSessionDeps + ?Sized>(&self, deps: &Arc<D>) {
        for (kind, selected) in [("microphone", &self.input), ("speaker", &self.output)] {
            if let Some(saved) = &selected.saved_missing {
                tracing::warn!(kind, saved = %saved, using = %selected.label,
                    "saved audio device not connected; using the system default");
                deps.emit_system_alert(
                    format!("Your {kind} isn't connected"),
                    format!(
                        "Your chosen {kind} isn't connected, so Rekindle is using \"{}\". It \
                         switches back when it is plugged in.",
                        selected.label
                    ),
                );
            }
        }
    }
}

pub struct DeviceMonitorParams<D: VoiceSessionDeps + ?Sized> {
    pub device_error_rx: mpsc::Receiver<String>,
    /// Cancelled when the loop's session scope shuts down.
    pub stop: tokio_util::sync::CancellationToken,
    pub deps: Arc<D>,
}

pub async fn run<D: VoiceSessionDeps + ?Sized>(mut params: DeviceMonitorParams<D>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!("device monitor loop started");

    loop {
        tokio::select! {
            biased;

            () = params.stop.cancelled() => {
                tracing::info!("device monitor loop: shutdown signal received");
                break;
            }

            Some(error_msg) = params.device_error_rx.recv() => {
                tracing::warn!(error = %error_msg, "device monitor: cpal stream error detected");
                // The open device failed: reopen on whatever the call should
                // use now, even if that is the same name.
                if let Err(e) = reselect(&params.deps, true).await {
                    tracing::error!(error = %e, "device monitor: reopen after stream error failed");
                }
                // restart_loops spawned a fresh monitor — this instance must exit.
                break;
            }

            _ = tick.tick() => {
                match reselect(&params.deps, false).await {
                    Ok(false) => {}
                    Ok(true) => break,
                    Err(e) => {
                        tracing::error!(error = %e, "device monitor: device switch failed");
                        break;
                    }
                }
            }
        }
    }

    tracing::info!("device monitor loop exited");
}

/// A device change stopped the call's audio and could not start it again:
/// say so in the log and to the user, who is otherwise left in a silent
/// call (Jitsi Meet raises a warning notification for every microphone
/// error, `base/devices/middleware.web.ts`).
pub(crate) fn report_reopen_failure<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    error: &VoiceError,
) {
    tracing::error!(%error, "audio devices not reopened after a device change; the call has no audio");
    deps.emit_system_alert(
        "Audio couldn't restart".to_string(),
        format!(
            "Rekindle couldn't reopen your audio devices ({error}). Leave and rejoin the call."
        ),
    );
}

/// Reopen on the devices the call should use if they differ from the open
/// ones (or always, after a stream error). Returns whether it reopened; the
/// caller (the monitor loop) MUST exit then — `restart_loops` spawns a
/// fresh monitor to replace it.
async fn reselect<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    force: bool,
) -> Result<bool, VoiceError> {
    if !deps.voice_engine_present() {
        return Ok(false);
    }
    let prefs = deps.audio_prefs();
    let targets = AudioTargets::select(&prefs)?;
    let open = deps.voice_engine_device_config();
    if !force && targets.match_open(&open) {
        return Ok(false);
    }

    // LOOPS_ONLY: don't stop the monitor — we ARE the monitor; awaiting
    // our own JoinHandle would deadlock.
    crate::session::shutdown::shutdown_voice(deps, &VoiceShutdownOpts::LOOPS_ONLY).await;
    deps.stop_audio_devices();
    deps.set_voice_engine_devices(
        Some(targets.input.id.clone()),
        Some(targets.output.id.clone()),
    );
    deps.set_voice_engine_input_channels(prefs.input_channels.clone());
    crate::session::restart::restart_loops(deps)
        .await
        .inspect_err(|e| report_reopen_failure(deps, e))?;

    for (kind, selected, was, saved) in [
        ("input", &targets.input, &open.0, &prefs.input_device),
        ("output", &targets.output, &open.1, &prefs.output_device),
    ] {
        if was.as_deref() == Some(selected.id.as_str()) {
            continue;
        }
        let reason = if selected.saved_missing.is_some() {
            "saved device disconnected"
        } else if saved.as_deref() == Some(selected.id.as_str()) {
            "saved device reconnected"
        } else {
            "system default changed"
        };
        tracing::info!(kind, device = %selected.label, reason, "device monitor: switched device");
        deps.emit_device_changed(kind.to_string(), selected.label.clone(), reason.to_string());
        if selected.saved_missing.is_none() {
            deps.emit_system_alert(
                "Audio device switched".to_string(),
                format!("Now using \"{}\" for {kind} ({reason}).", selected.label),
            );
        }
    }
    targets.announce_missing(deps);
    Ok(true)
}
