//! Fallback capture backend for non-Linux targets built WITHOUT the
//! `native-capture` feature — reports the stack unavailable while exposing
//! the same API surface, so the `src-tauri` facade never platform- or
//! feature-branches.

use tokio::sync::mpsc;

use crate::shared::{CaptureConfig, CaptureError, EncodedFrame, PreviewFrame, VideoDevice};

/// No native capture backend was built.
pub fn capture_available() -> bool {
    false
}

/// No devices without a backend.
pub fn list_devices() -> Vec<VideoDevice> {
    Vec::new()
}

/// A never-constructed session — `start` always reports unavailability.
/// Present so `NativeCaptureSession` and its control surface resolve on
/// this platform exactly as on the native path.
pub struct NativeCaptureSession;

#[allow(
    clippy::unused_self,
    reason = "the unavailable stub mirrors the native NativeCaptureSession control surface so the src-tauri facade never feature-branches; there is no session state to act on"
)]
impl NativeCaptureSession {
    pub fn start(
        _config: &CaptureConfig,
        _frame_tx: mpsc::Sender<EncodedFrame>,
        _preview_tx: mpsc::Sender<PreviewFrame>,
        _error_tx: mpsc::Sender<String>,
    ) -> Result<Self, CaptureError> {
        Err(CaptureError::Unavailable(
            "native capture not built — enable the `native-capture` feature".into(),
        ))
    }

    pub fn set_bitrate_kbps(&self, _kbps: u32) {}

    pub fn force_keyframe(&self) {}

    pub fn stop(self) {}
}
