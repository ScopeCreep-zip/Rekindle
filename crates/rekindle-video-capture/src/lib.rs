//! Native camera capture + VP9 realtime CBR encode, cross-platform on GStreamer.
//!
//! ONE pipeline, all platforms. A `GstDeviceMonitor`-selected camera
//! source — `v4l2src`/`pipewiresrc` on Linux, `avfvideosrc` on macOS,
//! `ksvideosrc`/`mfvideosrc` on Windows, chosen by the OS's own GStreamer
//! device provider — feeds `decodebin ! videoconvert ! I420 ! tee`, fanning
//! out to a `vp9enc deadline=1 end-usage=cbr` peer-egress branch and a
//! `jpegenc` self-view branch. GStreamer owns capability negotiation,
//! MJPEG/NV12/YUYV→I420 conversion, scaling, and realtime-CBR VP9 encode on
//! every OS — the reason this crate exists. No per-OS Rust capture code and
//! no thin FFI-callback binding (the source element is picked by the OS's
//! GStreamer provider), so there is no cross-platform capability negotiator
//! or callback-panic surface to own.
//!
//! Exposes `capture_available()`, `list_devices()`, and a
//! `NativeCaptureSession` (`start`/`stop`/`set_bitrate_kbps`/
//! `force_keyframe`) producing `EncodedFrame` (peer egress) and
//! `PreviewFrame` (self-view), so the `src-tauri` `native_video` facade
//! never platform-branches. Requires the GStreamer runtime present per
//! platform (Linux system packages; macOS/Windows bundled in the installer).

mod device;
mod error;
mod pipeline;

pub use device::{list_devices, VideoDevice};
pub use error::CaptureError;
pub use pipeline::{
    capture_available, CaptureConfig, EncodedFrame, NativeCaptureSession, PreviewFrame,
};
