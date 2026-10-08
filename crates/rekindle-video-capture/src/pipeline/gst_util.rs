//! Small GStreamer helpers shared by [`super::start`]: element creation
//! bound to a pipeline, `vp9enc` rate-control property wiring, and bus
//! `Error` message classification.

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::error::CaptureError;

use super::CaptureConfig;

pub(super) fn make(pipeline: &gst::Pipeline, factory: &str) -> Result<gst::Element, CaptureError> {
    let el = gst::ElementFactory::make(factory)
        .build()
        .map_err(|e| CaptureError::Unavailable(format!("{factory}: {e}")))?;
    pipeline
        .add(&el)
        .map_err(|e| CaptureError::Pipeline(format!("add {factory}: {e}")))?;
    Ok(el)
}

pub(super) fn configure_encoder(encoder: &gst::Element, config: &CaptureConfig) {
    // deadline=1 µs selects VPX_DL_REALTIME — the entire point.
    encoder.set_property("deadline", 1i64);
    encoder.set_property_from_str("end-usage", "cbr");
    encoder.set_property(
        "target-bitrate",
        i32::try_from(u64::from(config.start_bitrate_kbps) * 1000).unwrap_or(i32::MAX),
    );
    encoder.set_property(
        "keyframe-max-dist",
        i32::try_from(config.keyframe_max_dist).unwrap_or(128),
    );
    // Realtime knobs, shared by vp8enc/vp9enc (both GstVPXEnc): no
    // lookahead, fast speed preset, bounded worst-case quality, short
    // rate-control buffer so CBR is enforced over ~0.5 s not 6 s,
    // error-resilient partitions for lossy transport.
    //
    // Keyframe SIZE bounding (plan R4): GstVPXEnc exposes no libvpx
    // max-intra knob — intra size is bounded only indirectly by the
    // short CBR buffer model below; the pacer's oversized-keyframe
    // intake guard remains the explicit detector.
    //
    // cpu-used range differs (vp8enc 0..16, vp9enc −16..16); 8 is valid
    // for both — fast realtime on either encoder.
    encoder.set_property("lag-in-frames", 0i32);
    encoder.set_property("cpu-used", 8i32);
    encoder.set_property("max-quantizer", 56i32);
    encoder.set_property("buffer-size", 500i32);
    encoder.set_property("buffer-initial-size", 300i32);
    encoder.set_property("buffer-optimal-size", 400i32);
    encoder.set_property("threads", 2i32);
    encoder.set_property_from_str("error-resilient", "default");
}

pub(super) fn map_bus_error(err: &gst::message::Error, source_desc: &str) -> CaptureError {
    let inner = err.error();
    if inner.matches(gst::ResourceError::Busy) {
        CaptureError::Busy(format!("{source_desc} is in use by another application"))
    } else if inner.matches(gst::ResourceError::NotFound)
        || inner.matches(gst::ResourceError::OpenRead)
        || inner.matches(gst::ResourceError::OpenReadWrite)
    {
        CaptureError::Device(format!("{source_desc}: {inner}"))
    } else {
        CaptureError::Pipeline(format!("{source_desc}: {inner}"))
    }
}
