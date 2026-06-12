//! Native Linux camera capture + VP8 realtime encode (GStreamer).
//!
//! Exists because the webview encode path is structurally broken on
//! Linux: WebKitGTK maps WebCodecs VP9 `realtime` onto libvpx's
//! GOOD-quality deadline (no true CBR — observed 4-6× bitrate
//! overshoot with 100-280 KB frames), and no WebCodecs knob fixes it.
//! GStreamer's `vp8enc` with `deadline=1 end-usage=cbr` runs libvpx's
//! real RTC rate-control path; the wire stays VP8, which every
//! receiving platform's WebCodecs decoder already handles via the
//! per-fragment codec tag.
//!
//! Linux-only by design (macOS/Windows webview encoders honor their
//! bitrates); the whole module tree is `#[cfg(target_os = "linux")]`
//! and the crate compiles to an empty lib elsewhere — the src-tauri
//! `native_video` facade provides cross-platform stubs.

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
pub use pipeline::{capture_available, CaptureConfig, EncodedFrame, NativeCaptureSession};
