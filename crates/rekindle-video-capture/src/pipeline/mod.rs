//! GStreamer capture+encode pipeline with an in-process `tee` fan-out:
//!
//! ```text
//! {camera source} ! video/x-raw;image/jpeg ! decodebin ! videoconvert ! video/x-raw,format=I420 ! tee t.
//!   t. ! queue leaky=downstream ! videoscale ! videorate
//!      ! video/x-raw,format=I420,width=W,height=H,framerate=F/1
//!      ! vp9enc deadline=1 end-usage=cbr target-bitrate=B ... ! appsink   (→ peers)
//!   t. ! queue leaky=downstream ! videoscale ! videorate
//!      ! video/x-raw,format=I420,width=320,height=180,framerate=F/1
//!      ! jpegenc ! appsink                                                 (→ self-view)
//! ```
//!
//! ONE camera open (raw `v4l2src`), fanned out the way every native P2P
//! client does it (Jami's observer set, qTox's `CameraSource` signal,
//! Linphone's MSFilter `tee`): one branch encodes VP9 for peers, the
//! other emits small JPEG stills the webview paints to a canvas as the
//! LOCAL self-view. Never a second `getUserMedia` consumer (Linux V4L2
//! forbids two openers of one camera; PipeWire multiplexing is fragile)
//! and never an encode→decode loopback (an anti-pattern used by no real
//! app). `decodebin` absorbs the MJPEG-vs-raw camera split; the leaky
//! queues back-pressure on RAW frames so encoded output is never
//! dropped; `vp9enc deadline=1 end-usage=cbr` is libvpx's true RTC
//! rate-control path — the entire reason this crate exists.

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_video as gst_video;

mod gst_util;
mod probe;
mod start;

#[cfg(test)]
mod tests;

pub use probe::capture_available;

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Persisted camera display label (the same label getUserMedia
    /// reports); `None` = first available device.
    pub device_label: Option<String>,
    /// Element factory override for hermetic tests (`videotestsrc`).
    /// Production callers leave this `None`.
    pub source_override: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub start_bitrate_kbps: u32,
    /// Encoder keyframe ceiling in FRAMES (`keyframe-max-dist`) — the
    /// app cadence + keyframe-request path force keyframes earlier.
    pub keyframe_max_dist: u32,
}

/// One encoded VP9 chunk for the peer egress branch.
#[derive(Debug)]
pub struct EncodedFrame {
    pub payload: Vec<u8>,
    pub keyframe: bool,
    /// When the camera captured the frame, on the wall clock (Unix ms):
    /// the buffer's running time, as the live source stamped it, carried
    /// back from the sink's current running time. Receivers measure lip
    /// sync against the audio's wall-clock capture stamps.
    pub capture_wall_ms: u64,
}

/// One JPEG still from the preview branch — the LOCAL self-view. Small
/// (≈320×180) and codec-stateless, so the webview paints it straight to
/// a canvas via `createImageBitmap`, with no WebCodecs decoder involved.
#[derive(Debug)]
pub struct PreviewFrame {
    pub jpeg: Vec<u8>,
}

/// The largest camera mode the source may negotiate. Encode runs at
/// 854×480@15 (the session's video constraints); a 720p capture is a 1.5×
/// downscale. Production capturers choose a camera mode near the request
/// rather than the largest (libwebrtc `GetBestMatchedCapability`, Chromium
/// and Firefox fitness distance + `activeFormat`); in GStreamer that choice
/// is expressed as a capsfilter after the live source, through which "the
/// caps information flow proceeds from the user, through the potential caps
/// of the source" (gst-docs design/negotiation.md), and the source fixates
/// among its real modes. Left open, `avfvideosrc` fixates height to the
/// maximum and `v4l2src` toward 3840×2160, whatever the encode needs.
const CAPTURE_MAX_WIDTH: i32 = 1280;
const CAPTURE_MAX_HEIGHT: i32 = 720;
const CAPTURE_MAX_FPS: i32 = 30;

/// The source capsfilter: raw first, then MJPEG (filter order is preference
/// order), each bounded by the capture ceiling, with no caps features so
/// GLMemory modes cannot negotiate (avfvideosrc offers them first; their
/// buffers do not link to `videoconvert`'s system-memory sink).
pub(super) fn source_constraints() -> gst::Caps {
    let bounded = |media: &str| {
        gst::Structure::builder(media)
            .field("width", gst::IntRange::new(1, CAPTURE_MAX_WIDTH))
            .field("height", gst::IntRange::new(1, CAPTURE_MAX_HEIGHT))
            .field(
                "framerate",
                gst::FractionRange::new(
                    gst::Fraction::new(1, 1),
                    gst::Fraction::new(CAPTURE_MAX_FPS, 1),
                ),
            )
            .build()
    };
    gst::Caps::builder_full()
        .structure(bounded("video/x-raw"))
        .structure(bounded("image/jpeg"))
        .build()
}

/// Preview self-view dimensions — a small thumbnail; the heavy lifting
/// (resolution, bitrate) is the encode branch's job.
const PREVIEW_WIDTH: i32 = 320;
const PREVIEW_HEIGHT: i32 = 180;

/// A running capture session. Dropping it without `stop()` still tears
/// the pipeline down (Drop impl) — `stop()` exists for explicit,
/// logged teardown.
#[derive(Debug)]
pub struct NativeCaptureSession {
    pipeline: gst::Pipeline,
    encoder: gst::Element,
    /// Identifies the source in errors ("Integrated RGB Camera").
    source_desc: String,
}

impl NativeCaptureSession {
    /// Follow the bitrate policy's encoder-domain target. `vp9enc`
    /// applies rate-control properties to an initialized encoder live
    /// (`vpx_codec_enc_config_set` — de-facto stable 1.20→main).
    pub fn set_bitrate_kbps(&self, kbps: u32) {
        let bps = i32::try_from(u64::from(kbps) * 1000).unwrap_or(i32::MAX);
        self.encoder.set_property("target-bitrate", bps);
    }

    /// PLI-analog: force the next encoded frame to be a keyframe via
    /// the upstream force-key-unit event, injected at the pipeline's
    /// sink end so it travels upstream into the encoder's src pad —
    /// the path `GstVideoEncoder` handles it on. Callers throttle
    /// (the existing 300 ms keyframe floor).
    pub fn force_keyframe(&self) {
        let event = gst_video::UpstreamForceKeyUnitEvent::builder()
            .all_headers(true)
            .build();
        if !self.pipeline.send_event(event) {
            tracing::warn!(
                target: "rekindle_video_capture",
                "force-key-unit event not handled"
            );
        }
    }

    pub fn stop(self) {
        tracing::info!(
            target: "rekindle_video_capture",
            source = %self.source_desc,
            "native capture pipeline stopping"
        );
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Drop for NativeCaptureSession {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
