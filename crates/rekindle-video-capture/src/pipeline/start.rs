//! Pipeline construction and the blocking start-confirmation state
//! machine: build the decode→tee→{encode,preview} graph, wire the
//! appsink callbacks, reach PLAYING, and block until the first encoded
//! sample (success) or a bus error / deadline (failure).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::device::create_source;
use crate::error::CaptureError;

use super::gst_util::{configure_encoder, make, map_bus_error};
use super::{CaptureConfig, EncodedFrame, NativeCaptureSession, PreviewFrame};
use super::{PREVIEW_HEIGHT, PREVIEW_WIDTH};

/// How long a starting session may run without producing a sample or
/// a bus error before we call it dead. V4L2 reports a busy device
/// asynchronously ~tens of ms AFTER the pipeline reaches PLAYING, so
/// success must be judged on the first sample, never the state change.
const START_DEADLINE: Duration = Duration::from_secs(2);

impl NativeCaptureSession {
    /// Build, start, and CONFIRM the pipeline: returns only after the
    /// first encoded sample (success) or a bus error / deadline
    /// (failure). Blocking — callers run it on a blocking thread.
    pub fn start(
        config: &CaptureConfig,
        frame_tx: tokio::sync::mpsc::Sender<EncodedFrame>,
        preview_tx: tokio::sync::mpsc::Sender<PreviewFrame>,
        error_tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<Self, CaptureError> {
        gst::init().map_err(|e| CaptureError::Unavailable(e.to_string()))?;

        let (source, source_desc) = match &config.source_override {
            // Tests pass element descriptions ("videotestsrc
            // num-buffers=10") — parse_bin handles properties; a bare
            // factory name builds directly.
            Some(desc) if desc.contains(' ') => (
                gst::parse::bin_from_description(desc, true)
                    .map_err(|e| CaptureError::Unavailable(format!("{desc}: {e}")))?
                    .upcast::<gst::Element>(),
                desc.clone(),
            ),
            Some(name) => (
                gst::ElementFactory::make(name)
                    .build()
                    .map_err(|e| CaptureError::Unavailable(format!("{name}: {e}")))?,
                name.clone(),
            ),
            None => create_source(config.device_label.as_deref())?,
        };

        let pipeline = gst::Pipeline::new();
        // Shared head: decode (MJPEG-or-raw) → convert → system-memory
        // I420 → tee. Pinning a plain `video/x-raw` (no `memory:DMABuf`
        // feature) here forces a system-memory copy the CPU branches can
        // negotiate, and normalizing to I420 once means both branches
        // inherit it (vp9enc wants I420; jpegenc accepts it).
        let decode = make(&pipeline, "decodebin")?;
        let convert = make(&pipeline, "videoconvert")?;
        let head_caps = make(&pipeline, "capsfilter")?;
        let tee = make(&pipeline, "tee")?;
        // Encode branch (→ peers): scale/rate to the negotiated encode
        // shape, then vp9enc. Cameras run fixed mode sets — videorate
        // adapts the delivered framerate to the encode fps.
        let enc_queue = make(&pipeline, "queue")?;
        let enc_scale = make(&pipeline, "videoscale")?;
        let enc_rate = make(&pipeline, "videorate")?;
        let enc_caps = make(&pipeline, "capsfilter")?;
        let encoder = make(&pipeline, "vp9enc")?;
        let enc_appsink_el = make(&pipeline, "appsink")?;
        // Preview branch (→ local self-view): downscale to a small
        // thumbnail and JPEG-encode it.
        let pv_queue = make(&pipeline, "queue")?;
        let pv_scale = make(&pipeline, "videoscale")?;
        let pv_rate = make(&pipeline, "videorate")?;
        let pv_caps = make(&pipeline, "capsfilter")?;
        let jpegenc = make(&pipeline, "jpegenc")?;
        let pv_appsink_el = make(&pipeline, "appsink")?;
        pipeline
            .add(&source)
            .map_err(|e| CaptureError::Pipeline(e.to_string()))?;

        let fps = i32::try_from(config.fps).unwrap_or(15);
        head_caps.set_property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .build(),
        );
        enc_caps.set_property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", i32::try_from(config.width).unwrap_or(854))
                .field("height", i32::try_from(config.height).unwrap_or(480))
                .field("framerate", gst::Fraction::new(fps, 1))
                .build(),
        );
        pv_caps.set_property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", PREVIEW_WIDTH)
                .field("height", PREVIEW_HEIGHT)
                .field("framerate", gst::Fraction::new(fps, 1))
                .build(),
        );

        // Raw-side backpressure on BOTH branches: drop CAPTURED frames
        // under load, never encoded ones (an encoded drop is a
        // reference-chain break). The post-tee queue also gives each
        // branch its own streaming thread so one can't stall the other.
        for q in [&enc_queue, &pv_queue] {
            q.set_property_from_str("leaky", "downstream");
            q.set_property("max-size-buffers", 2u32);
            q.set_property("max-size-bytes", 0u32);
            q.set_property("max-size-time", 0u64);
        }

        configure_encoder(&encoder, config);
        // jpegenc quality 0..100; 50 keeps the self-view thumbnail at a
        // few KB/frame.
        jpegenc.set_property("quality", 50i32);

        source
            .link(&decode)
            .map_err(|e| CaptureError::Pipeline(format!("source!decodebin: {e}")))?;
        gst::Element::link_many([&convert, &head_caps, &tee])
            .map_err(|e| CaptureError::Pipeline(format!("convert..tee: {e}")))?;
        gst::Element::link_many([
            &enc_queue,
            &enc_scale,
            &enc_rate,
            &enc_caps,
            &encoder,
            &enc_appsink_el,
        ])
        .map_err(|e| CaptureError::Pipeline(format!("encode branch: {e}")))?;
        gst::Element::link_many([
            &pv_queue,
            &pv_scale,
            &pv_rate,
            &pv_caps,
            &jpegenc,
            &pv_appsink_el,
        ])
        .map_err(|e| CaptureError::Pipeline(format!("preview branch: {e}")))?;
        // Linking from the tee auto-requests a fresh src pad per branch.
        tee.link(&enc_queue)
            .map_err(|e| CaptureError::Pipeline(format!("tee!encode: {e}")))?;
        tee.link(&pv_queue)
            .map_err(|e| CaptureError::Pipeline(format!("tee!preview: {e}")))?;

        // decodebin pads appear per-stream at runtime — link the first
        // video pad to videoconvert.
        let convert_weak = convert.downgrade();
        decode.connect_pad_added(move |_, pad| {
            let Some(convert) = convert_weak.upgrade() else {
                return;
            };
            let sink = convert
                .static_pad("sink")
                .expect("videoconvert always has a sink pad");
            if sink.is_linked() {
                return;
            }
            if let Err(e) = pad.link(&sink) {
                tracing::warn!(target: "rekindle_video_capture", error = %e, "decodebin pad link failed");
            }
        });

        // Success is judged on the ENCODE branch's first sample — peers
        // are the priority; the preview branch is best-effort.
        let first_sample = Arc::new(AtomicBool::new(false));
        let enc_sink = enc_appsink_el
            .clone()
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| CaptureError::Pipeline("encode appsink cast".into()))?;
        enc_sink.set_property("sync", false);
        let first_sample_cb = Arc::clone(&first_sample);
        enc_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let Ok(sample) = sink.pull_sample() else {
                        return Err(gst::FlowError::Eos);
                    };
                    let Some(buffer) = sample.buffer() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
                    let Ok(map) = buffer.map_readable() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    first_sample_cb.store(true, Ordering::Release);
                    // Streaming thread: never block. A full channel
                    // means the consumer stalled — dropping encoded
                    // frames here breaks the reference chain, but a
                    // stalled consumer already lost the stream; the
                    // keyframe-request path recovers.
                    let _ = frame_tx.try_send(EncodedFrame {
                        payload: map.as_slice().to_vec(),
                        keyframe,
                    });
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Preview branch: JPEG stills to the self-view. Pure best-effort
        // — a full channel (UI behind) just drops the still; the next one
        // is a fresh full frame (JPEG is intra-only, no reference chain).
        let pv_sink = pv_appsink_el
            .clone()
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| CaptureError::Pipeline("preview appsink cast".into()))?;
        pv_sink.set_property("sync", false);
        pv_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let Ok(sample) = sink.pull_sample() else {
                        return Err(gst::FlowError::Eos);
                    };
                    let Some(buffer) = sample.buffer() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let Ok(map) = buffer.map_readable() else {
                        return Ok(gst::FlowSuccess::Ok);
                    };
                    let _ = preview_tx.try_send(PreviewFrame {
                        jpeg: map.as_slice().to_vec(),
                    });
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| CaptureError::Pipeline(format!("set PLAYING: {e}")))?;

        // Start state machine: PLAYING means nothing yet — wait for the
        // first sample or the async bus verdict.
        let bus = pipeline.bus().ok_or_else(|| {
            let _ = pipeline.set_state(gst::State::Null);
            CaptureError::Pipeline("pipeline has no bus".into())
        })?;
        let deadline = Instant::now() + START_DEADLINE;
        // Error/Eos can land on the bus BEFORE the post-start watcher
        // exists (verified: a finite source posts EOS at ~40 ms) — the
        // start loop must forward what it sees, never pop-and-discard,
        // or an unplug racing the first sample vanishes.
        let mut pending_error: Option<String> = None;
        loop {
            if first_sample.load(Ordering::Acquire) {
                break;
            }
            if let Some(msg) = bus.timed_pop_filtered(
                Some(gst::ClockTime::from_mseconds(50)),
                &[gst::MessageType::Error, gst::MessageType::Eos],
            ) {
                match msg.view() {
                    gst::MessageView::Error(err) => {
                        let mapped = map_bus_error(err, &source_desc);
                        let _ = pipeline.set_state(gst::State::Null);
                        return Err(mapped);
                    }
                    gst::MessageView::Eos(_) => {
                        if first_sample.load(Ordering::Acquire) {
                            // Started, then immediately ended — the
                            // session is valid; the pump learns of the
                            // end through the error channel.
                            pending_error = Some("camera stream ended".into());
                            break;
                        }
                        let _ = pipeline.set_state(gst::State::Null);
                        return Err(CaptureError::Device(format!(
                            "{source_desc}: stream ended before the first frame"
                        )));
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                let _ = pipeline.set_state(gst::State::Null);
                return Err(CaptureError::Timeout(source_desc));
            }
        }
        if let Some(message) = pending_error {
            let _ = error_tx.try_send(message);
        }
        tracing::info!(
            target: "rekindle_video_capture",
            source = %source_desc,
            width = config.width,
            height = config.height,
            fps = config.fps,
            start_bitrate_kbps = config.start_bitrate_kbps,
            "native capture pipeline streaming"
        );

        // Post-start bus watcher: async errors (camera unplugged,
        // element failure) surface through error_tx for teardown.
        let bus_pipeline = pipeline.downgrade();
        let watcher_desc = source_desc.clone();
        std::thread::Builder::new()
            .name("native-video-bus".into())
            .spawn(move || loop {
                let Some(pipeline) = bus_pipeline.upgrade() else {
                    break;
                };
                let Some(bus) = pipeline.bus() else { break };
                drop(pipeline);
                if let Some(msg) = bus.timed_pop(Some(gst::ClockTime::from_mseconds(500))) {
                    match msg.view() {
                        gst::MessageView::Error(err) => {
                            let mapped = map_bus_error(err, &watcher_desc);
                            tracing::warn!(
                                target: "rekindle_video_capture",
                                error = %mapped,
                                "pipeline error after start"
                            );
                            let _ = error_tx.try_send(mapped.to_string());
                            break;
                        }
                        gst::MessageView::Eos(_) => {
                            let _ = error_tx.try_send("camera stream ended".into());
                            break;
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| CaptureError::Pipeline(format!("bus watcher spawn: {e}")))?;

        Ok(Self {
            pipeline,
            encoder,
            source_desc,
        })
    }
}
