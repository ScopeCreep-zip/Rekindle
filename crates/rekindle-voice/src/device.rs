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
