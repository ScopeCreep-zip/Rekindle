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

/// Find an audio device by name. A device that is not connected is an
/// error, never a silent substitute: which device to open when the saved
/// one is missing is [`select_device`]'s decision, made visibly before a
/// stream opens (plan C7.24).
pub fn find_device(
    host: &cpal::Host,
    name: &str,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    direction
        .devices(host)
        .into_iter()
        .find(|device| device.name().ok().as_deref() == Some(name))
        .ok_or_else(|| {
            VoiceError::AudioDevice(format!(
                "{} device \"{name}\" is not connected",
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
    /// The concrete device to open.
    pub name: String,
    /// The saved choice, when it is not connected and `name` is the
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
    saved: Option<&str>,
    direction: &DeviceDirection,
) -> Result<SelectedDevice, VoiceError> {
    let host = cpal::default_host();
    if let Some(name) = saved {
        if direction
            .devices(&host)
            .iter()
            .any(|device| device.name().ok().as_deref() == Some(name))
        {
            return Ok(SelectedDevice {
                name: name.to_string(),
                saved_missing: None,
            });
        }
    }
    let default = preferred_default_device(&host, direction)?;
    let name = default.name().map_err(|e| {
        VoiceError::AudioDevice(format!(
            "{} default device has no name: {e}",
            direction.label()
        ))
    })?;
    Ok(SelectedDevice {
        name,
        saved_missing: saved.map(str::to_string),
    })
}

/// Resolve an audio device by optional name for the given direction.
///
/// - `Some(name)` → exactly that device ([`find_device`]).
/// - `None` → the preferred default (see [`preferred_default_device`]).
pub fn resolve_device(
    host: &cpal::Host,
    device_name: Option<&str>,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    match device_name {
        Some(name) => find_device(host, name, direction),
        None => preferred_default_device(host, direction),
    }
}

/// The default device to open when the user hasn't pinned a specific one.
///
/// On a sound-server Linux stack (PipeWire/PulseAudio) the raw ALSA `default`
/// PCM is backed by the hardware device the server holds exclusively, so cpal's
/// blocking `snd_pcm_open`/`snd_pcm_start` inside `build_*_stream` can hang
/// indefinitely. The `pipewire`/`pulse` ALSA bridge PCMs are non-exclusive
/// client connections to the server and open without blocking, so we prefer
/// them over `default` and only fall back to the raw default when no bridge is
/// present (e.g. a bare-ALSA system). Other platforms use the system default
/// directly. This mirrors cpal's own documented Linux workaround for versions
/// without a native PipeWire backend.
fn preferred_default_device(
    host: &cpal::Host,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    #[cfg(target_os = "linux")]
    {
        for bridge in ["pipewire", "pulse"] {
            if let Some(device) = direction
                .devices(host)
                .into_iter()
                .find(|device| device.name().ok().as_deref() == Some(bridge))
            {
                tracing::info!(
                    device = bridge,
                    direction = direction.label(),
                    "using sound-server bridge device instead of raw ALSA default"
                );
                return Ok(device);
            }
        }
    }

    direction.default_device(host).ok_or_else(|| {
        VoiceError::AudioDevice(format!("no {} device available", direction.label()))
    })
}

/// Enumerated audio devices (input and output).
pub struct EnumeratedDevices {
    /// Input devices: `(name, is_default, channels)`, `channels` being what
    /// an input-channel choice picks from.
    pub input_devices: Vec<(String, bool, u16)>,
    /// Output devices: `(name, is_default)`.
    pub output_devices: Vec<(String, bool)>,
}

/// Collect `(name, is_default)` pairs for the devices in a direction.
fn collect_device_names(
    host: &cpal::Host,
    direction: &DeviceDirection,
    default_name: Option<&str>,
) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    for device in direction.devices(host) {
        if let Ok(name) = device.name() {
            let is_default = default_name == Some(name.as_str());
            result.push((name, is_default));
        }
    }
    result
}

/// Enumerate all available audio input and output devices.
pub fn enumerate_audio_devices() -> EnumeratedDevices {
    let host = cpal::default_host();
    let default_input_name = DeviceDirection::Input
        .default_device(&host)
        .and_then(|d| d.name().ok());
    let default_output_name = DeviceDirection::Output
        .default_device(&host)
        .and_then(|d| d.name().ok());

    EnumeratedDevices {
        input_devices: DeviceDirection::Input
            .devices(&host)
            .into_iter()
            .filter_map(|device| {
                let name = device.name().ok()?;
                let is_default = default_input_name.as_deref() == Some(name.as_str());
                Some((name, is_default, max_input_channels(&device)))
            })
            .collect(),
        output_devices: collect_device_names(
            &host,
            &DeviceDirection::Output,
            default_output_name.as_deref(),
        ),
    }
}
