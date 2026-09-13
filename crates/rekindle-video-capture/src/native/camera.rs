//! nokhwa device discovery + camera open (AVFoundation / Media Foundation).
//!
//! macOS note: camera authorization (the `NSCameraUsageDescription` prompt /
//! `AVCaptureDevice.requestAccess`) is the `src-tauri` shell's job — the
//! next integration step. Here, an unauthorized or busy device surfaces as
//! a mapped [`CaptureError`] from `Camera::new`/`open_stream`, never a panic.

use std::sync::OnceLock;

use nokhwa::utils::{ApiBackend, CameraInfo, FrameFormat, RequestedFormat, RequestedFormatType};
use nokhwa::{query, Camera, NokhwaError};

use super::select;
use crate::shared::{CaptureConfig, CaptureError, VideoDevice};

#[cfg(target_os = "macos")]
const BACKEND: ApiBackend = ApiBackend::AVFoundation;
#[cfg(target_os = "windows")]
const BACKEND: ApiBackend = ApiBackend::MediaFoundation;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const BACKEND: ApiBackend = ApiBackend::Auto;

/// Every nokhwa `FrameFormat` our converter handles (`session::to_i420`:
/// NV12/YUYV take a direct repack, all others decode through
/// `RgbFormat`). Passed as the decoder set on BOTH the permissive open and
/// the `Exact` re-select so nokhwa's format filter never empties: the open
/// then resolves against any device mode, and the re-select accepts whatever
/// native mode the scorer picked from the device's own enumeration.
const ALL_FRAME_FORMATS: &[FrameFormat] = &[
    FrameFormat::NV12,
    FrameFormat::YUYV,
    FrameFormat::RAWRGB,
    FrameFormat::RAWBGR,
    FrameFormat::GRAY,
    FrameFormat::MJPEG,
];

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

/// Open the configured (or first) camera, negotiate a NATIVE mode nearest
/// the encode target (enumerate → score → select → set), and start
/// streaming. Returns the live camera plus its display label. Runs on the
/// capture thread — the nokhwa camera never leaves it.
///
/// The old code asked the camera for the ENCODE target
/// (`Closest(854×480, NV12)`), which fails on macOS: 854 is never a sensor
/// width and nokhwa's `Closest` filters by an exact `FrameFormat` first, so
/// an NV12-absent (or narrower) device empties the filter and returns
/// `None` → "Cannot fulfill request". Instead we open permissively, pull the
/// device's real modes, and let [`select::select_capture_format`] pick one
/// (libwebrtc `GetBestMatchedCapability`); `convert.rs`/`scale.rs` bring that
/// native mode DOWN to the encode target.
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

    // 1. Open PERMISSIVELY. `AbsoluteHighestResolution` resolves to a real
    //    device mode (unlike `Closest`, which gates on an exact
    //    `FrameFormat`). The full `ALL_FRAME_FORMATS` decoder set keeps even
    //    a device whose top-resolution mode is an unusual format from
    //    failing nokhwa's post-resolution format filter. This mode is
    //    transient — step 3 re-selects before the stream opens.
    let opened = RequestedFormat::with_formats(
        RequestedFormatType::AbsoluteHighestResolution,
        ALL_FRAME_FORMATS,
    );
    let mut camera = Camera::new(info.index().clone(), opened).map_err(|e| map_open(&e))?;

    // 2. Enumerate the device's REAL capabilities.
    let formats = camera.compatible_camera_formats().unwrap_or_default();

    // 3. Score + select the native mode nearest the encode target, then pin
    //    it. `set_camera_requset` is the non-deprecated setter
    //    (`set_camera_format` is deprecated → -D warnings); `Exact` over the
    //    full format set always accepts a mode that came from the device's
    //    own enumeration. If the device enumerated nothing or the scorer
    //    returned `None`, keep whatever `AbsoluteHighestResolution` opened —
    //    it is a valid mode — and never fail here.
    if let Some(best) =
        select::select_capture_format(&formats, config.width, config.height, config.fps)
    {
        let request =
            RequestedFormat::with_formats(RequestedFormatType::Exact(best), ALL_FRAME_FORMATS);
        camera
            .set_camera_requset(request)
            .map_err(|e| map_open(&e))?;
    }

    // 4. Observability: log the FINAL negotiated mode against the target, so
    //    the next live capture log shows exactly what was selected.
    let negotiated = camera.camera_format();
    tracing::info!(
        target: "rekindle_video_capture",
        device = %desc,
        selected_width = negotiated.width(),
        selected_height = negotiated.height(),
        selected_fps = negotiated.frame_rate(),
        selected_format = %negotiated.format(),
        target_width = config.width,
        target_height = config.height,
        target_fps = config.fps,
        "negotiated camera capture format"
    );

    // 5. Start streaming at the negotiated mode.
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
