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
///
/// Builds a raw `v4l2src` bound to the resolved `/dev/videoN`. This is
/// the single-consumer capture model every native P2P client uses
/// (Jami, qTox via FFmpeg-v4l2; Linphone via `msv4l2`): we are the SOLE
/// opener of the camera and fan frames out IN-PROCESS via the pipeline
/// `tee` (one branch → VP9 to peers, one → JPEG self-view). We do NOT
/// route through `pipewiresrc` — that path existed only to share the
/// camera with a second WebView `getUserMedia` consumer, which we no
/// longer do (and which Linux V4L2 forbids / PipeWire makes fragile).
/// PipeWire opens the v4l2 device on-demand, so it's free for a direct
/// `v4l2src` open whenever nothing else is streaming — exactly our case.
///
/// The GstDevice (whichever provider found it — v4l2 or pipewire) still
/// drives label matching and yields the `/dev/videoN` path from its
/// properties; if no v4l2 path is exposed we fall back to the monitor's
/// own `create_element`.
pub(crate) fn create_source(
    saved_label: Option<&str>,
) -> Result<(gst::Element, String), CaptureError> {
    let monitor = monitor()?;
    let devices = monitor.devices();
    monitor.stop();

    // Candidates matching the saved label, else every camera. A single
    // physical camera can appear more than once (v4l2 + pipewire
    // providers, or capture vs metadata nodes) under the same name.
    let candidates: Vec<gst::Device> = match saved_label {
        Some(label) => {
            let matched: Vec<gst::Device> = devices
                .iter()
                .filter(|d| d.display_name() == label)
                .cloned()
                .collect();
            if matched.is_empty() {
                devices.iter().cloned().collect()
            } else {
                matched
            }
        }
        None => devices.iter().cloned().collect(),
    };
    if candidates.is_empty() {
        return Err(CaptureError::Device("no camera devices found".into()));
    }

    // Prefer a device that exposes a v4l2 capture path → build v4l2src
    // on it directly. Fall back to the monitor's create_element for a
    // device with no readable path.
    let mut fallback: Option<(gst::Element, String)> = None;
    for device in &candidates {
        let desc = device.display_name().to_string();
        if let Some(path) = v4l2_device_path(device) {
            let element = gst::ElementFactory::make("v4l2src")
                .property("device", &path)
                .build()
                .map_err(|e| CaptureError::Unavailable(format!("v4l2src {path}: {e}")))?;
            return Ok((element, desc));
        }
        if fallback.is_none() {
            if let Ok(element) = device.create_element(None) {
                fallback = Some((element, desc));
            }
        }
    }
    fallback.ok_or_else(|| CaptureError::Device("no usable camera source".into()))
}

/// Read the `/dev/videoN` path a GstDevice points at, across provider
/// quirks: the v4l2 provider sets `device.path`; the PipeWire provider
/// sets `api.v4l2.path` (and `object.path` as `v4l2:/dev/videoN`).
fn v4l2_device_path(device: &gst::Device) -> Option<String> {
    let props = device.properties()?;
    for key in ["device.path", "api.v4l2.path"] {
        if let Ok(path) = props.get::<String>(key) {
            if path.starts_with("/dev/video") {
                return Some(path);
            }
        }
    }
    // PipeWire `object.path` is `v4l2:/dev/videoN` — strip the scheme.
    if let Ok(object_path) = props.get::<String>("object.path") {
        if let Some(path) = object_path.strip_prefix("v4l2:") {
            if path.starts_with("/dev/video") {
                return Some(path.to_string());
            }
        }
    }
    None
}
