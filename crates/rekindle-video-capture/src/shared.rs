//! Cross-platform (non-Linux) capture vocabulary shared by the nokhwa
//! native backend and the unavailable stub. Field-for-field the same shape
//! as the Linux GStreamer path's types (`pipeline/mod.rs`, `error.rs`,
//! `device.rs`) so the `src-tauri` `native_video` facade never
//! platform-branches on them.

use thiserror::Error;

/// Capture session configuration — the same shape as the GStreamer path's
/// `CaptureConfig`, so a single caller builds it on every platform.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Persisted camera display label (the label getUserMedia reports);
    /// `None` = first available device.
    pub device_label: Option<String>,
    /// Element-factory override for the GStreamer path's hermetic tests.
    /// Unused by the native (nokhwa) backend; kept for API symmetry so the
    /// facade constructs one `CaptureConfig` on every platform.
    pub source_override: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub start_bitrate_kbps: u32,
    /// Keyframe ceiling in FRAMES: the native loop forces a keyframe every
    /// `keyframe_max_dist` frames (the libvpx encoder exposes no
    /// periodic-keyframe knob), and the keyframe-request path forces
    /// earlier ones on demand.
    pub keyframe_max_dist: u32,
}

/// One encoded VP9 chunk for the peer egress branch. Wire timestamps are
/// stamped by the consumer (the native pump uses wall-clock ms).
#[derive(Debug)]
pub struct EncodedFrame {
    pub payload: Vec<u8>,
    pub keyframe: bool,
}

/// One JPEG still from the preview branch — the LOCAL self-view. Small and
/// codec-stateless, so the webview paints it straight to a canvas.
#[derive(Debug)]
pub struct PreviewFrame {
    pub jpeg: Vec<u8>,
}

/// A camera device entry for the settings UI.
#[derive(Debug, Clone)]
pub struct VideoDevice {
    pub display_name: String,
}

/// Native capture failure taxonomy — the same variant names as the
/// GStreamer path's `CaptureError`, so error mapping in the facade is
/// uniform across platforms.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// The capture stack is unavailable (feature not built, no backend).
    #[error("capture stack unavailable: {0}")]
    Unavailable(String),
    /// The camera is owned by another streaming consumer.
    #[error("camera busy: {0}")]
    Busy(String),
    /// Device missing/unreadable (unplugged, permissions).
    #[error("camera unavailable: {0}")]
    Device(String),
    /// Opened but produced no frame inside the start deadline.
    #[error("camera timeout: {0}")]
    Timeout(String),
    /// Any other capture/encode failure.
    #[error("pipeline error: {0}")]
    Pipeline(String),
}
