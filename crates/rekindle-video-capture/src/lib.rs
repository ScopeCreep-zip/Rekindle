//! Native camera capture + VP9 realtime encode, per platform:
//!
//! - **Linux** (`#[cfg(target_os = "linux")]`, always): a GStreamer
//!   `v4l2src` pipeline. WebKitGTK maps WebCodecs VP9 `realtime` onto
//!   libvpx's GOOD-quality deadline (no true CBR — 4-6× bitrate overshoot),
//!   so the webview encode path is structurally broken there; `vp9enc
//!   deadline=1 end-usage=cbr` runs libvpx's real RTC rate-control path.
//! - **macOS / Windows** with the `native-capture` feature: nokhwa
//!   (AVFoundation / Media Foundation) capture → `yuv` colour convert →
//!   planar scale → `rekindle-video-libvpx` VP9 CBR. The webview encoders
//!   honour their bitrates on those platforms, so this is opt-in.
//! - **otherwise** (non-Linux, feature off): an unavailable stub.
//!
//! Every path exposes the SAME surface — `capture_available()`,
//! `list_devices()`, and a `NativeCaptureSession` with
//! `start`/`stop`/`set_bitrate_kbps`/`force_keyframe` producing
//! `EncodedFrame` (peer egress) and `PreviewFrame` (self-view) — so the
//! `src-tauri` `native_video` facade never platform-branches.

// ── Linux: GStreamer ─────────────────────────────────────────────
#[cfg(target_os = "linux")]
mod device;
#[cfg(target_os = "linux")]
mod error;
#[cfg(target_os = "linux")]
mod pipeline;

#[cfg(target_os = "linux")]
pub use device::{list_devices, VideoDevice};
#[cfg(target_os = "linux")]
pub use error::CaptureError;
#[cfg(target_os = "linux")]
pub use pipeline::{
    capture_available, CaptureConfig, EncodedFrame, NativeCaptureSession, PreviewFrame,
};

// ── Non-Linux: shared capture vocabulary (native backend + stub) ──
#[cfg(not(target_os = "linux"))]
mod shared;
#[cfg(not(target_os = "linux"))]
pub use shared::{CaptureConfig, CaptureError, EncodedFrame, PreviewFrame, VideoDevice};

// ── macOS / Windows: nokhwa + libvpx native backend ──────────────
#[cfg(all(feature = "native-capture", not(target_os = "linux")))]
mod native;
#[cfg(all(feature = "native-capture", not(target_os = "linux")))]
pub use native::{capture_available, list_devices, NativeCaptureSession};

// ── Fallback: unavailable stub ───────────────────────────────────
#[cfg(all(not(feature = "native-capture"), not(target_os = "linux")))]
mod stub;
#[cfg(all(not(feature = "native-capture"), not(target_os = "linux")))]
pub use stub::{capture_available, list_devices, NativeCaptureSession};
