//! GStreamer capture+encode pipeline:
//!
//! ```text
//! {camera source} ! decodebin ! videoconvert ! videoscale ! videorate
//!   ! video/x-raw,format=I420,width=W,height=H,framerate=F/1
//!   ! queue leaky=downstream max-size-buffers=2
//!   ! vp8enc deadline=1 end-usage=cbr target-bitrate=B ...
//!   ! appsink
//! ```
//!
//! `decodebin` absorbs the MJPEG-vs-raw camera split (PipeWire does
//! not transparently decode MJPEG — the app owns that); the leaky
//! queue back-pressures on RAW frames so encoded output is never
//! dropped; `vp8enc deadline=1 end-usage=cbr` is libvpx's true RTC
//! rate-control path — the entire reason this crate exists.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::device::create_source;
use crate::error::CaptureError;

/// How long a starting session may run without producing a sample or
/// a bus error before we call it dead. V4L2 reports a busy device
/// asynchronously ~tens of ms AFTER the pipeline reaches PLAYING, so
/// success must be judged on the first sample, never the state change.
const START_DEADLINE: Duration = Duration::from_secs(2);

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

/// One encoded VP8 chunk. Wire timestamps are stamped by the consumer
/// (the native pump uses wall-clock ms so the loopback latency probe
/// measures true encode→paint) — pipeline running time stays internal.
#[derive(Debug)]
pub struct EncodedFrame {
    pub payload: Vec<u8>,
    pub keyframe: bool,
}

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
    /// Build, start, and CONFIRM the pipeline: returns only after the
    /// first encoded sample (success) or a bus error / deadline
    /// (failure). Blocking — callers run it on a blocking thread.
    pub fn start(
        config: &CaptureConfig,
        frame_tx: tokio::sync::mpsc::Sender<EncodedFrame>,
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
        let decode = make(&pipeline, "decodebin")?;
        let convert = make(&pipeline, "videoconvert")?;
        let scale = make(&pipeline, "videoscale")?;
        // Cameras (and videotestsrc) run fixed mode sets — videorate
        // adapts whatever framerate the device delivers to the encode
        // fps, or the capsfilter would simply refuse to negotiate.
        let rate = make(&pipeline, "videorate")?;
        let capsfilter = make(&pipeline, "capsfilter")?;
        let queue = make(&pipeline, "queue")?;
        let encoder = make(&pipeline, "vp8enc")?;
        let appsink_el = make(&pipeline, "appsink")?;
        pipeline
            .add(&source)
            .map_err(|e| CaptureError::Pipeline(e.to_string()))?;

        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "I420")
            .field("width", i32::try_from(config.width).unwrap_or(854))
            .field("height", i32::try_from(config.height).unwrap_or(480))
            .field(
                "framerate",
                gst::Fraction::new(i32::try_from(config.fps).unwrap_or(15), 1),
            )
            .build();
        capsfilter.set_property("caps", &caps);

        // Raw-side backpressure: drop CAPTURED frames under load, never
        // encoded ones (an encoded drop is a reference-chain break).
        queue.set_property_from_str("leaky", "downstream");
        queue.set_property("max-size-buffers", 2u32);
        queue.set_property("max-size-bytes", 0u32);
        queue.set_property("max-size-time", 0u64);

        configure_encoder(&encoder, config);

        source
            .link(&decode)
            .map_err(|e| CaptureError::Pipeline(format!("source!decodebin: {e}")))?;
        gst::Element::link_many([&convert, &scale, &rate, &capsfilter, &queue, &encoder, &appsink_el])
            .map_err(|e| CaptureError::Pipeline(format!("convert..appsink: {e}")))?;

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

        let first_sample = Arc::new(AtomicBool::new(false));
        let sink = appsink_el
            .clone()
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| CaptureError::Pipeline("appsink cast".into()))?;
        sink.set_property("sync", false);
        let first_sample_cb = Arc::clone(&first_sample);
        sink.set_callbacks(
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

    /// Follow the bitrate policy's encoder-domain target. `vp8enc`
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

fn make(pipeline: &gst::Pipeline, factory: &str) -> Result<gst::Element, CaptureError> {
    let el = gst::ElementFactory::make(factory)
        .build()
        .map_err(|e| CaptureError::Unavailable(format!("{factory}: {e}")))?;
    pipeline
        .add(&el)
        .map_err(|e| CaptureError::Pipeline(format!("add {factory}: {e}")))?;
    Ok(el)
}

fn configure_encoder(encoder: &gst::Element, config: &CaptureConfig) {
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
    // Realtime knobs (verified stable 1.20→main): no lookahead, fast
    // speed preset, bounded worst-case quality, short rate-control
    // buffer so CBR is enforced over ~0.5 s not 6 s, error-resilient
    // partitions for lossy transport.
    //
    // Keyframe SIZE bounding (plan R4 "investigate in Phase 2"):
    // GstVPXEnc exposes no libvpx max-intra knob — the verified
    // 47-property surface (identical 1.20→main) has nothing mapping to
    // rc_max_intra_bitrate_pct. Intra size is therefore bounded only
    // indirectly by the short CBR buffer model below; the pacer's
    // oversized-keyframe intake guard remains the explicit detector.
    encoder.set_property("lag-in-frames", 0i32);
    encoder.set_property("cpu-used", 8i32);
    encoder.set_property("max-quantizer", 56i32);
    encoder.set_property("buffer-size", 500i32);
    encoder.set_property("buffer-initial-size", 300i32);
    encoder.set_property("buffer-optimal-size", 400i32);
    encoder.set_property("threads", 2i32);
    encoder.set_property_from_str("error-resilient", "default");
}

fn map_bus_error(err: &gst::message::Error, source_desc: &str) -> CaptureError {
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

/// The R1 four-step availability checklist, cached: (1) gst::init,
/// (2) every required element factory exists (incl. at least one
/// camera source), (3) the vp8enc properties we set exist on the
/// class, (4) a one-shot videotestsrc→vp8enc dry-run reaches PAUSED —
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
        "vp8enc",
        "jpegdec",
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
    let encoder = gst::ElementFactory::make("vp8enc")
        .build()
        .map_err(|e| format!("vp8enc instantiate: {e}"))?;
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
            return Err(format!("vp8enc missing property: {prop}"));
        }
    }
    // Dry run: plugin files can exist while their shared-library deps
    // are broken (partial installs) — only a state change proves it.
    let pipeline = gst::parse::launch("videotestsrc num-buffers=1 ! vp8enc deadline=1 ! fakesink")
        .map_err(|e| format!("dry-run parse: {e}"))?;
    pipeline
        .set_state(gst::State::Paused)
        .map_err(|e| format!("dry-run pause: {e}"))?;
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(bitrate_kbps: u32) -> CaptureConfig {
        CaptureConfig {
            device_label: None,
            source_override: Some("videotestsrc".into()),
            width: 320,
            height: 240,
            fps: 15,
            start_bitrate_kbps: bitrate_kbps,
            keyframe_max_dist: 60,
        }
    }

    fn drain_for(
        rx: &mut tokio::sync::mpsc::Receiver<EncodedFrame>,
        duration: Duration,
    ) -> Vec<EncodedFrame> {
        let deadline = Instant::now() + duration;
        let mut frames = Vec::new();
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(f) => frames.push(f),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        frames
    }

    #[test]
    fn frames_flow_and_first_is_keyframe() {
        if !capture_available() {
            eprintln!("skipping: gstreamer unavailable in this environment");
            return;
        }
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
        let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
        let session = NativeCaptureSession::start(&test_config(300), frame_tx, error_tx)
            .expect("videotestsrc pipeline starts");
        let frames = drain_for(&mut frame_rx, Duration::from_millis(1_200));
        session.stop();
        assert!(frames.len() >= 5, "got {} frames", frames.len());
        assert!(frames[0].keyframe, "first encoded frame is a keyframe");
    }

    #[test]
    fn force_keyframe_yields_keyframe_quickly() {
        if !capture_available() {
            eprintln!("skipping: gstreamer unavailable in this environment");
            return;
        }
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
        let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
        let session = NativeCaptureSession::start(&test_config(300), frame_tx, error_tx)
            .expect("videotestsrc pipeline starts");
        // Let the stream settle past its initial keyframe.
        let _ = drain_for(&mut frame_rx, Duration::from_millis(500));
        session.force_keyframe();
        let after = drain_for(&mut frame_rx, Duration::from_millis(600));
        session.stop();
        // keyframe-max-dist=60 at 15 fps = 4 s natural cadence; the
        // initial keyframe was at t≈0 and the force at t≈0.5 s, so ANY
        // keyframe in this window is attributable to the force. The
        // window may also start with pre-force in-flight deltas.
        let kinds: Vec<bool> = after.iter().map(|f| f.keyframe).collect();
        assert!(
            kinds.contains(&true),
            "keyframe within 600 ms of the force: {kinds:?}"
        );
    }

    #[test]
    fn bitrate_halving_shrinks_output() {
        if !capture_available() {
            eprintln!("skipping: gstreamer unavailable in this environment");
            return;
        }
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(1024);
        let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
        let session = NativeCaptureSession::start(&test_config(600), frame_tx, error_tx)
            .expect("videotestsrc pipeline starts");
        let high: usize = drain_for(&mut frame_rx, Duration::from_millis(1_500))
            .iter()
            .map(|f| f.payload.len())
            .sum();
        session.set_bitrate_kbps(120);
        // Settle, then measure.
        let _ = drain_for(&mut frame_rx, Duration::from_millis(500));
        let low: usize = drain_for(&mut frame_rx, Duration::from_millis(1_500))
            .iter()
            .map(|f| f.payload.len())
            .sum();
        session.stop();
        assert!(
            low * 2 < high,
            "120 kbps window ({low} B) should be well under half the 600 kbps window ({high} B)"
        );
    }

    #[test]
    fn post_start_failure_fires_error_channel() {
        // Plan Phase 2 test (d): the ASYNC error path — a finite
        // source ends the stream after start succeeded; the bus
        // watcher must surface it through error_tx (the same path a
        // camera unplug takes).
        if !capture_available() {
            eprintln!("skipping: gstreamer unavailable in this environment");
            return;
        }
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(256);
        let (error_tx, mut error_rx) = tokio::sync::mpsc::channel(4);
        let mut config = test_config(300);
        config.source_override = Some("videotestsrc num-buffers=10".into());
        let session = NativeCaptureSession::start(&config, frame_tx, error_tx)
            .expect("finite videotestsrc starts");
        let _ = drain_for(&mut frame_rx, Duration::from_millis(300));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut message = None;
        while Instant::now() < deadline {
            if let Ok(m) = error_rx.try_recv() {
                message = Some(m);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        session.stop();
        let message = message.expect("error channel fires after the stream ends");
        assert!(
            message.contains("ended"),
            "EOS surfaces as a stream-ended error: {message}"
        );
    }

    #[test]
    fn broken_pipeline_reports_unavailable() {
        if gst::init().is_err() {
            return;
        }
        let (frame_tx, _frame_rx) = tokio::sync::mpsc::channel(4);
        let (error_tx, _error_rx) = tokio::sync::mpsc::channel(4);
        let mut config = test_config(300);
        config.source_override = Some("no-such-element-exists".into());
        let err = NativeCaptureSession::start(&config, frame_tx, error_tx).unwrap_err();
        assert!(matches!(err, CaptureError::Unavailable(_)), "{err}");
    }
}
