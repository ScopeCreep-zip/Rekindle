//! macOS/Windows native capture: nokhwa (AVFoundation / Media Foundation)
//! → `yuv` colour convert → planar scale → `rekindle-video-libvpx` VP9 CBR
//! → `EncodedFrame` (peer egress) + JPEG `PreviewFrame` (self-view).
//!
//! Exposes the same `NativeCaptureSession` surface as the Linux GStreamer
//! path so the `src-tauri` facade never platform-branches. The camera and
//! encoder live on a dedicated OS thread (the `cpal` posture
//! `rekindle-voice` uses for its !Send audio streams); only compressed
//! frames leave the process.

mod camera;
mod convert;
mod preview;
mod scale;
mod session;

pub use camera::{capture_available, list_devices};
pub use session::NativeCaptureSession;
