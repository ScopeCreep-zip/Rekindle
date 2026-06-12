//! Camera device discovery + label-based resolution.
//!
//! Selection is keyed by DISPLAY LABEL, matching the persisted
//! `videoDeviceLabel` the settings UI already stores (the same label
//! getUserMedia reports — PipeWire carries it as `api.v4l2.cap.card`,
//! GstDeviceMonitor as the device display name). The monitor's
//! `Device::create_element` hands back a correctly-configured source
//! for whichever provider found it (v4l2 or pipewire), so the
//! pipeline never hardcodes a source element for real cameras.

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::error::CaptureError;

#[derive(Debug, Clone)]
pub struct VideoDevice {
    pub display_name: String,
}

fn monitor() -> Result<gst::DeviceMonitor, CaptureError> {
    gst::init().map_err(|e| CaptureError::Unavailable(e.to_string()))?;
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    monitor
        .start()
        .map_err(|e| CaptureError::Unavailable(format!("device monitor: {e}")))?;
    Ok(monitor)
}

/// Enumerate camera sources for the settings UI.
pub fn list_devices() -> Vec<VideoDevice> {
    let Ok(monitor) = monitor() else {
        return Vec::new();
    };
    let devices = monitor
        .devices()
        .iter()
        .map(|d| VideoDevice {
            display_name: d.display_name().to_string(),
        })
        .collect();
    monitor.stop();
    devices
}

/// Resolve the saved label (or the first available camera) to a
/// configured source element + its description for error messages.
pub(crate) fn create_source(
    saved_label: Option<&str>,
) -> Result<(gst::Element, String), CaptureError> {
    let monitor = monitor()?;
    let devices = monitor.devices();
    let device = saved_label
        .and_then(|label| {
            devices
                .iter()
                .find(|d| d.display_name() == label)
                .cloned()
        })
        .or_else(|| devices.front().cloned());
    monitor.stop();
    let Some(device) = device else {
        return Err(CaptureError::Device("no camera devices found".into()));
    };
    let desc = device.display_name().to_string();
    let element = device
        .create_element(None)
        .map_err(|e| CaptureError::Device(format!("{desc}: {e}")))?;
    Ok((element, desc))
}
