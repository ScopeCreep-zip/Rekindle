use cpal::traits::{DeviceTrait, HostTrait};

use crate::VoiceError;

/// Whether to search for an input or output device.
pub enum DeviceDirection {
    Input,
    Output,
}

/// Find an audio device by name, falling back to the default for that direction.
pub fn find_device(
    host: &cpal::Host,
    name: &str,
    direction: &DeviceDirection,
) -> Result<cpal::Device, VoiceError> {
    let is_input = matches!(direction, DeviceDirection::Input);
    let label = if is_input { "input" } else { "output" };

    let devices: Vec<cpal::Device> = if is_input {
        host.input_devices().into_iter().flatten().collect()
    } else {
        host.output_devices().into_iter().flatten().collect()
    };

    for device in devices {
        if device.name().ok().as_deref() == Some(name) {
            return Ok(device);
        }
    }

    tracing::warn!(device = %name, direction = label, "requested device not found — falling back to default");
    let default = if is_input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    default.ok_or_else(|| VoiceError::AudioDevice(format!("no {label} device available")))
}

/// Enumerated audio devices (input and output).
pub struct EnumeratedDevices {
    /// Input devices: `(name, is_default)`.
    pub input_devices: Vec<(String, bool)>,
    /// Output devices: `(name, is_default)`.
    pub output_devices: Vec<(String, bool)>,
}

/// Enumerate all available audio input and output devices.
pub fn enumerate_audio_devices() -> Result<EnumeratedDevices, VoiceError> {
    let host = cpal::default_host();
    let default_input_name = host.default_input_device().and_then(|d| d.name().ok());
    let default_output_name = host.default_output_device().and_then(|d| d.name().ok());

    let mut input_devices = Vec::new();
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                let is_default = default_input_name.as_deref() == Some(&name);
                input_devices.push((name, is_default));
            }
        }
    }

    let mut output_devices = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                let is_default = default_output_name.as_deref() == Some(&name);
                output_devices.push((name, is_default));
            }
        }
    }

    Ok(EnumeratedDevices {
        input_devices,
        output_devices,
    })
}
