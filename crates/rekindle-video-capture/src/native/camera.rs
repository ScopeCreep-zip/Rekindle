//! nokhwa device discovery + camera open (AVFoundation / Media Foundation).
//!
//! macOS note: camera authorization (the `NSCameraUsageDescription` prompt /
//! `AVCaptureDevice.requestAccess`) is the `src-tauri` shell's job — the
//! next integration step. Here, an unauthorized or busy device surfaces as
//! a mapped [`CaptureError`] from `Camera::new`/`open_stream`, never a panic.

use std::sync::OnceLock;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraInfo, FrameFormat, RequestedFormat, RequestedFormatType,
    Resolution,
};
use nokhwa::{query, Camera, NokhwaError};

use crate::shared::{CaptureConfig, CaptureError, VideoDevice};

#[cfg(target_os = "macos")]
const BACKEND: ApiBackend = ApiBackend::AVFoundation;
#[cfg(target_os = "windows")]
const BACKEND: ApiBackend = ApiBackend::MediaFoundation;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const BACKEND: ApiBackend = ApiBackend::Auto;

/// Enumerate cameras, guarding nokhwa's no-device `query()` panic
/// (l1npengtul/nokhwa#192) so probing a machine with no camera returns an
/// empty list instead of unwinding.
fn query_devices() -> Vec<CameraInfo> {
    std::panic::catch_unwind(|| query(BACKEND).unwrap_or_default()).unwrap_or_default()
}

/// True when at least one camera is queryable — cached process-wide (the
/// facade gate the frontend asks instead of sniffing the OS).
pub fn capture_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| !query_devices().is_empty())
}

/// Camera display labels for the settings UI.
pub fn list_devices() -> Vec<VideoDevice> {
    query_devices()
        .into_iter()
        .map(|info| VideoDevice {
            display_name: info.human_name(),
        })
        .collect()
}

/// Open the configured (or first) camera at the closest mode to the
/// requested resolution/fps, preferring NV12, and start streaming. Returns
/// the live camera plus its display label. Runs on the capture thread — the
/// nokhwa camera never leaves it.
pub(crate) fn open_camera(config: &CaptureConfig) -> Result<(Camera, String), CaptureError> {
    let devices = query_devices();
    let info = match &config.device_label {
        Some(label) => devices
            .iter()
            .find(|d| d.human_name() == *label)
            .or_else(|| devices.first())
            .cloned(),
        None => devices.first().cloned(),
    }
    .ok_or_else(|| CaptureError::Device("no camera devices found".into()))?;

    let desc = info.human_name();
    let requested =
        RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(config.width, config.height),
            FrameFormat::NV12,
            config.fps,
        )));
    let mut camera = Camera::new(info.index().clone(), requested).map_err(|e| map_open(&e))?;
    camera.open_stream().map_err(|e| map_open(&e))?;
    Ok((camera, desc))
}

/// Map a nokhwa open/stream error onto the capture taxonomy.
fn map_open(err: &NokhwaError) -> CaptureError {
    let msg = err.to_string();
    let low = msg.to_lowercase();
    if low.contains("permission") || low.contains("denied") || low.contains("authoriz") {
        CaptureError::Device(format!("camera permission denied: {msg}"))
    } else if low.contains("in use") || low.contains("busy") {
        CaptureError::Busy(msg)
    } else {
        CaptureError::Pipeline(msg)
    }
}
