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

/// Find an audio device by name, falling back to the default for that direction.
pub fn find_device(
    host: &cpal::Host,
    name: &str,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    for device in direction.devices(host) {
        if device.name().ok().as_deref() == Some(name) {
            return Ok(device);
        }
    }

    tracing::warn!(
        device = %name,
        direction = direction.label(),
        "requested device not found — falling back to default"
    );
    direction
        .default_device(host)
        .ok_or_else(|| VoiceError::AudioDevice(format!("no {} device available", direction.label())))
}

/// Resolve an audio device by optional name for the given direction.
///
/// - `Some(name)` → search by name, fall back to default (via `find_device`).
/// - `None` → return the system default directly.
pub fn resolve_device(
    host: &cpal::Host,
    device_name: Option<&str>,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    match device_name {
        Some(name) => find_device(host, name, direction),
        None => direction.default_device(host).ok_or_else(|| {
            VoiceError::AudioDevice(format!("no {} device available", direction.label()))
        }),
    }
}

/// Enumerated audio devices (input and output).
pub struct EnumeratedDevices {
    /// Input devices: `(name, is_default)`.
    pub input_devices: Vec<(String, bool)>,
    /// Output devices: `(name, is_default)`.
    pub output_devices: Vec<(String, bool)>,
}

/// Collect `(name, is_default)` pairs for all devices in the given direction.
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
pub fn enumerate_audio_devices() -> Result<EnumeratedDevices, VoiceError> {
    let host = cpal::default_host();
    let default_input_name = DeviceDirection::Input
        .default_device(&host)
        .and_then(|d| d.name().ok());
    let default_output_name = DeviceDirection::Output
        .default_device(&host)
        .and_then(|d| d.name().ok());

    Ok(EnumeratedDevices {
        input_devices: collect_device_names(
            &host,
            &DeviceDirection::Input,
            default_input_name.as_deref(),
        ),
        output_devices: collect_device_names(
            &host,
            &DeviceDirection::Output,
            default_output_name.as_deref(),
        ),
    })
}
