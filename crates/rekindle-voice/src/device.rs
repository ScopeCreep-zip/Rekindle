use cpal::traits::{DeviceTrait, HostTrait};

use crate::VoiceError;

/// Whether to search for an input or output device.
pub enum DeviceDirection {
    Input,
    Output,
}

impl DeviceDirection {
    /// "input" or "output" — for error messages and tracing.
    fn label(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    /// List all devices for this direction from the host.
    fn devices(&self, host: &cpal::Host) -> Vec<cpal::Device> {
        match self {
            Self::Input => host.input_devices().into_iter().flatten().collect(),
            Self::Output => host.output_devices().into_iter().flatten().collect(),
        }
    }

    /// Get the system default device for this direction.
    fn default_device(&self, host: &cpal::Host) -> Option<cpal::Device> {
        match self {
            Self::Input => host.default_input_device(),
            Self::Output => host.default_output_device(),
        }
    }
}

/// A stream's error callback. A device that went away, changed under the
/// stream (the system default moved) or invalidated it is handed to the
/// device monitor, which reselects and reopens (plan C7.24): the host's
/// own notifications, as Jitsi Meet follows `devicechange` instead of
/// polling. An underrun is the stream recovering on its own (cpal 0.18
/// reports it as an error) and is only logged.
pub fn on_stream_error(
    direction: &'static str,
    err: &cpal::Error,
    error_tx: &std::sync::mpsc::Sender<String>,
) {
    if err.kind() == cpal::ErrorKind::Xrun {
        tracing::debug!(direction, error = %err, "audio stream xrun");
        return;
    }
    tracing::warn!(direction, kind = ?err.kind(), error = %err, "audio stream error");
    let _ = error_tx.send(format!("{direction}: {err}"));
}

/// A device's stable identifier: cpal's `DeviceId` in its string form,
/// which survives reconnects and reboots and round-trips through
/// `FromStr` (cpal 0.17+). Saved choices and per-device settings are keyed
/// by it; the description is only for display.
pub fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

/// A device's human-readable name, for logs and the settings list.
pub fn device_label(device: &cpal::Device) -> String {
    device.description().map_or_else(
        |_| device_id(device).unwrap_or_else(|| "unnamed".into()),
        |d| d.name().to_string(),
    )
}

/// Find an audio device by id. A device that is not connected is an
/// error, never a silent substitute: which device to open when the saved
/// one is missing is [`select_device`]'s decision, made visibly before a
/// stream opens (plan C7.24).
pub fn find_device(
    host: &cpal::Host,
    id: &str,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    direction
        .devices(host)
        .into_iter()
        .find(|device| device_id(device).as_deref() == Some(id))
        .ok_or_else(|| {
            VoiceError::AudioDevice(format!(
                "{} device \"{id}\" is not connected",
                direction.label()
            ))
        })
}

/// The most input channels `device` offers (1 when it reports none) — what
/// an input-channel choice picks from (plan C7.24b).
pub fn max_input_channels(device: &cpal::Device) -> u16 {
    device
        .supported_input_configs()
        .map(|ranges| ranges.map(|r| r.channels()).max().unwrap_or(1))
        .unwrap_or(1)
        .max(1)
}

/// The device a saved choice resolves to right now (plan C7.24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedDevice {
    /// The id of the concrete device to open.
    pub id: String,
    /// Its display name, for what the user is told.
    pub label: String,
    /// The saved choice, when it is not connected and `id` is the
    /// system default standing in for it. The choice itself is kept.
    pub saved_missing: Option<String>,
}

/// Which device to open for a saved choice: the saved device when it is
/// connected, otherwise the system default, with the saved choice kept so
/// the device monitor switches back when it returns. This is Jitsi Meet's
/// policy (`getUserSelectedMicDeviceId` resolves the stored device or
/// yields the default; `getNewAudioInputDevice` switches back to the
/// stored device when it is plugged in) and the W3C `getUserMedia`
/// guidance (request the stored `deviceId` without `exact`, so an absent
/// device is replaced rather than failing).
///
/// # Errors
/// No device exists for the direction at all.
pub fn select_device(
    host: &cpal::Host,
    saved: Option<&str>,
    direction: &DeviceDirection,
) -> Result<SelectedDevice, VoiceError> {
    if let Some(id) = saved {
        if let Some(device) = direction
            .devices(host)
            .iter()
            .find(|device| device_id(device).as_deref() == Some(id))
        {
            return Ok(SelectedDevice {
                id: id.to_string(),
                label: device_label(device),
                saved_missing: None,
            });
        }
    }
    let default = direction.default_device(host).ok_or_else(|| {
        VoiceError::AudioDevice(format!("no {} device available", direction.label()))
    })?;
    let id = device_id(&default).ok_or_else(|| {
        VoiceError::AudioDevice(format!("{} default device has no id", direction.label()))
    })?;
    Ok(SelectedDevice {
        id,
        label: device_label(&default),
        saved_missing: saved.map(str::to_string),
    })
}

/// Resolve an audio device by optional id for the given direction.
///
/// - `Some(id)` → exactly that device ([`find_device`]).
/// - `None` → the host's default device. On Linux the host is PipeWire or
///   PulseAudio when one is running (cpal's host order), so the default is
///   the sound server's, never a raw ALSA PCM.
pub fn resolve_device(
    host: &cpal::Host,
    device_id: Option<&str>,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    match device_id {
        Some(id) => find_device(host, id, direction),
        None => direction.default_device(host).ok_or_else(|| {
            VoiceError::AudioDevice(format!("no {} device available", direction.label()))
        }),
    }
}

/// One device in a settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedDevice {
    /// Stable id ([`device_id`]): what a choice saves.
    pub id: String,
    /// Display name.
    pub label: String,
    pub is_default: bool,
}

/// Enumerated audio devices (input and output).
pub struct EnumeratedDevices {
    /// Input devices with their channel counts, which an input-channel
    /// choice picks from.
    pub input_devices: Vec<(ListedDevice, u16)>,
    pub output_devices: Vec<ListedDevice>,
}

fn listed(device: &cpal::Device, default_id: Option<&str>) -> Option<ListedDevice> {
    let id = device_id(device)?;
    Some(ListedDevice {
        is_default: default_id == Some(id.as_str()),
        label: device_label(device),
        id,
    })
}

/// Enumerate all available audio input and output devices.
pub fn enumerate_audio_devices() -> EnumeratedDevices {
    let host = cpal::default_host();
    let default_input = DeviceDirection::Input
        .default_device(&host)
        .as_ref()
        .and_then(device_id);
    let default_output = DeviceDirection::Output
        .default_device(&host)
        .as_ref()
        .and_then(device_id);
    EnumeratedDevices {
        input_devices: DeviceDirection::Input
            .devices(&host)
            .iter()
            .filter_map(|d| listed(d, default_input.as_deref()).map(|l| (l, max_input_channels(d))))
            .collect(),
        output_devices: DeviceDirection::Output
            .devices(&host)
            .iter()
            .filter_map(|d| listed(d, default_output.as_deref()))
            .collect(),
    }
}
