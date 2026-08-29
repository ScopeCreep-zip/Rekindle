//! Native-capture availability probing, cached process-wide.

use std::sync::OnceLock;

use gstreamer as gst;
use gstreamer::prelude::*;

/// The R1 four-step availability checklist, cached: (1) gst::init,
/// (2) every required element factory exists (incl. at least one
/// camera source), (3) the vp9enc properties we set exist on the
/// class, (4) a one-shot videotestsrc→vp9enc dry-run reaches PAUSED —
/// catching present-but-unloadable plugins. Any failure → false →
/// the webview path.
pub fn capture_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| match probe_available() {
        Ok(()) => true,
        Err(reason) => {
            tracing::info!(
                target: "rekindle_video_capture",
                %reason,
                "native capture unavailable — webview path stays active"
            );
            false
        }
    })
}

fn probe_available() -> Result<(), String> {
    gst::init().map_err(|e| format!("gst init: {e}"))?;
    for factory in [
        "vp9enc",
        "jpegdec",
        "jpegenc",
        "tee",
        "videoconvert",
        "videoscale",
        "videorate",
        "decodebin",
        "appsink",
        "queue",
        "capsfilter",
    ] {
        if gst::ElementFactory::find(factory).is_none() {
            return Err(format!("missing element: {factory}"));
        }
    }
    if gst::ElementFactory::find("pipewiresrc").is_none()
        && gst::ElementFactory::find("v4l2src").is_none()
    {
        return Err("no camera source element (pipewiresrc/v4l2src)".into());
    }
    let encoder = gst::ElementFactory::make("vp9enc")
        .build()
        .map_err(|e| format!("vp9enc instantiate: {e}"))?;
    for prop in [
        "deadline",
        "end-usage",
        "target-bitrate",
        "keyframe-max-dist",
        "lag-in-frames",
        "cpu-used",
        "max-quantizer",
        "buffer-size",
        "error-resilient",
    ] {
        if encoder.find_property(prop).is_none() {
            return Err(format!("vp9enc missing property: {prop}"));
        }
    }
    // Dry run: plugin files can exist while their shared-library deps
    // are broken (partial installs) — only a state change proves it.
    let pipeline = gst::parse::launch(
        "videotestsrc num-buffers=1 ! video/x-raw,format=I420 ! vp9enc deadline=1 ! fakesink",
    )
    .map_err(|e| format!("dry-run parse: {e}"))?;
    pipeline
        .set_state(gst::State::Paused)
        .map_err(|e| format!("dry-run pause: {e}"))?;
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}
