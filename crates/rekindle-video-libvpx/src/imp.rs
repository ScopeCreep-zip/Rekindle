//! Feature-ON path (`feature = "libvpx"`).
//!
//! The real VP8/VP9 **realtime** encoder, bound directly to libvpx through
//! `env-libvpx-sys` (`vpx_sys`). This is the one file in the crate that
//! carries the C dependency and the only place `unsafe` is used.
//!
//! # Why the raw FFI and not the `vpx-encode` wrapper
//!
//! `vpx-encode 0.6.2` exposes only `Encoder::new(Config{width,height,
//! timebase,bitrate,codec})` + `encode(pts,data)`. That surface cannot
//! set the three knobs a realtime call requires, which testing confirmed:
//!
//! - **`g_lag_in_frames = 0`** — without it VP9 buffers ~25 frames and
//!   `encode` returns nothing in realtime (VP8 defaults to 0, so it
//!   worked; VP9 did not). We set it to 0 so every frame emits at once.
//! - **`rc_end_usage = VPX_CBR`** — `vpx-encode` inherits libvpx's VBR
//!   default. CBR is the exact "quality fix" the plan calls for
//!   (`video-media-engine.md` §"Video — two separable problems": "A
//!   realtime call wants `deadline=REALTIME` and `end-usage=cbr` … what
//!   the GStreamer pipeline sets and what WebCodecs does not give us").
//! - **live `vpx_codec_enc_config_set`** — lets
//!   [`set_bitrate`](VideoEncoder::set_bitrate) retarget the encoder
//!   without tearing it down, and [`force_keyframe`](VideoEncoder::force_keyframe)
//!   use `VPX_EFLAG_FORCE_KF` per-frame — neither costs a keyframe or an
//!   encoder rebuild, unlike a `Config`-only wrapper would.
//!
//! Encode always uses `VPX_DL_REALTIME` with a high `cpu-used` (VP9 also
//! row-multithreaded), matching `rekindle-video-capture`'s `vp9enc
//! deadline=1 end-usage=cbr` GStreamer path — now on every platform.

use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr;

use rekindle_types::video::Codec;
use rekindle_video::codec::{EncodedVideoFrame, RawFrame, VideoEncoder};
use rekindle_video::error::VideoError;
use rekindle_video::{EncoderConstraints, VIDEO_START_KBPS};

use vpx_sys::{
    vp8e_enc_control_id::{VP8E_SET_CPUUSED, VP8E_SET_STATIC_THRESHOLD, VP9E_SET_ROW_MT},
    vpx_codec_control_, vpx_codec_ctx_t,
    vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT,
    vpx_codec_destroy, vpx_codec_enc_cfg_t, vpx_codec_enc_config_default, vpx_codec_enc_config_set,
    vpx_codec_enc_init_ver, vpx_codec_encode, vpx_codec_err_t, vpx_codec_error,
    vpx_codec_get_cx_data, vpx_codec_iter_t, vpx_codec_vp8_cx, vpx_codec_vp9_cx, vpx_image_t,
    vpx_img_fmt::VPX_IMG_FMT_I420,
    vpx_img_wrap,
    vpx_rc_mode::VPX_CBR,
    VPX_DL_REALTIME, VPX_EFLAG_FORCE_KF, VPX_ENCODER_ABI_VERSION, VPX_ERROR_RESILIENT_DEFAULT,
    VPX_FRAME_IS_KEY,
};

/// Framerate the timebase falls back to before `configure`, and the clamp
/// applied to a negotiated framerate. libvpx rate control reads the frame
/// cadence from `g_timebase`; we use `1/fps`.
const FPS_FALLBACK: u32 = 15;
const FPS_MIN: u32 = 1;
const FPS_MAX: u32 = 240;

/// Realtime `cpu-used` (speed/quality tradeoff). VP8 range is roughly
/// -16..16 (higher = faster); VP9 is 0..9. These pick fast realtime, the
/// same intent as the GStreamer path.
const CPU_USED_VP8: c_int = 12;
const CPU_USED_VP9: c_int = 8;

/// Map a non-OK libvpx status to a [`VideoError`]. The `vpx_codec_err_t`
/// variant name (e.g. `VPX_CODEC_INVALID_PARAM`) is self-describing.
fn err(res: vpx_codec_err_t) -> VideoError {
    VideoError::Encode(format!("libvpx: {res:?}"))
}

/// An initialized libvpx encoder context plus the config it was built
/// with (kept so `set_bitrate` can re-`set` it live). Its `Drop` runs the
/// matching `vpx_codec_destroy`.
struct VpxCtx {
    ctx: vpx_codec_ctx_t,
    cfg: vpx_codec_enc_cfg_t,
}

impl Drop for VpxCtx {
    fn drop(&mut self) {
        // SAFETY: `ctx` was initialized by `vpx_codec_enc_init_ver` (a
        // `VpxCtx` is only ever constructed after a successful init in
        // `rebuild`), and `vpx_codec_destroy` is its matching teardown.
        unsafe {
            vpx_codec_destroy(&raw mut self.ctx);
        }
    }
}

/// libvpx VP8/VP9 realtime encoder implementing
/// [`rekindle_video::codec::VideoEncoder`].
pub struct LibvpxEncoder {
    codec: Codec,
    constraints: Option<EncoderConstraints>,
    target_kbps: u32,
    fps: u32,
    /// The live encoder + its config, lazily (re)built on first encode and
    /// on a codec / dimension change.
    inner: Option<VpxCtx>,
    /// Dimensions the current `inner` was built at — a size change forces
    /// a rebuild.
    enc_dims: (u32, u32),
    /// Monotonic presentation timestamp, in timebase (1/fps) ticks. Reset
    /// to 0 whenever `inner` is rebuilt.
    next_pts: i64,
    /// A codec / dimension change is pending — the next encode rebuilds.
    needs_rebuild: bool,
    /// A keyframe was requested; the next encode passes `VPX_EFLAG_FORCE_KF`
    /// (no rebuild, no lost encoder state).
    force_kf_next: bool,
}

// SAFETY: `LibvpxEncoder` owns its libvpx encoder context exclusively and
// only ever touches it through `&mut self`, so it is never accessed
// concurrently. libvpx encoder contexts hold no caller-thread-local state
// and are sound to move between threads provided calls stay serialized —
// the same rationale `rekindle-voice` uses to move `cpal` streams onto
// dedicated threads. That makes the raw pointers inside `vpx_codec_ctx_t`
// (which mark it `!Send`) safe to send here.
unsafe impl Send for LibvpxEncoder {}

impl LibvpxEncoder {
    /// Create an encoder for `codec` (VP8 or VP9). The codec can be
    /// overridden by [`configure`](VideoEncoder::configure); H.264 is not
    /// a libvpx codec and is rejected at configure / encode time with
    /// [`VideoError::Unsupported`]. The target bitrate defaults to
    /// [`VIDEO_START_KBPS`] until [`set_bitrate`](VideoEncoder::set_bitrate).
    pub fn new(codec: Codec) -> Result<Self, VideoError> {
        // Reject an unsupported codec (H.264) up front, so `new` is
        // genuinely fallible and callers learn immediately.
        Self::iface(codec)?;
        Ok(Self {
            codec,
            constraints: None,
            target_kbps: VIDEO_START_KBPS,
            fps: FPS_FALLBACK,
            inner: None,
            enc_dims: (0, 0),
            next_pts: 0,
            needs_rebuild: true,
            force_kf_next: false,
        })
    }

    /// The libvpx encoder interface for our codec. H.264 has no libvpx
    /// encoder.
    fn iface(codec: Codec) -> Result<*const vpx_sys::vpx_codec_iface, VideoError> {
        let iface = match codec {
            // SAFETY: returns a pointer to libvpx's static VP8 encoder
            // interface singleton; no arguments, no invariants to uphold.
            Codec::Vp8 => unsafe { vpx_codec_vp8_cx() },
            // SAFETY: returns a pointer to libvpx's static VP9 encoder
            // interface singleton; no arguments, no invariants to uphold.
            Codec::Vp9 => unsafe { vpx_codec_vp9_cx() },
            Codec::H264 => {
                return Err(VideoError::Unsupported(
                    "libvpx encodes VP8/VP9 only — H.264 needs a different engine \
                     (WebCodecs/VideoToolbox/GStreamer)"
                        .to_string(),
                ))
            }
        };
        if iface.is_null() {
            return Err(VideoError::Encode(
                "libvpx returned a null codec interface".to_string(),
            ));
        }
        Ok(iface)
    }

    /// (Re)build the libvpx context for `width × height` at the current
    /// codec / bitrate / fps, configured for realtime CBR with zero
    /// encoder lag.
    fn rebuild(&mut self, width: u32, height: u32) -> Result<(), VideoError> {
        let iface = Self::iface(self.codec)?;

        // Start from libvpx's defaults, then override for realtime CBR.
        // The config struct holds C enum fields with no valid zero variant
        // (e.g. `g_bit_depth`), so it is initialized THROUGH a pointer by
        // `vpx_codec_enc_config_default` rather than materialized from
        // `mem::zeroed()` (which would be an invalid value).
        let mut cfg_uninit = std::mem::MaybeUninit::<vpx_codec_enc_cfg_t>::zeroed();
        // SAFETY: `iface` is a valid non-null interface pointer;
        // `config_default` writes a complete, valid config through the ptr.
        let res = unsafe { vpx_codec_enc_config_default(iface, cfg_uninit.as_mut_ptr(), 0) };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }
        // SAFETY: `config_default` returned VPX_CODEC_OK, so `cfg` is fully
        // initialized with valid enum discriminants.
        let mut cfg = unsafe { cfg_uninit.assume_init() };

        cfg.g_w = width;
        cfg.g_h = height;
        cfg.g_timebase.num = 1;
        cfg.g_timebase.den = c_int::try_from(self.fps).unwrap_or(15);
        cfg.g_error_resilient = VPX_ERROR_RESILIENT_DEFAULT;
        cfg.g_lag_in_frames = 0; // realtime: never buffer — emit every frame.
        cfg.g_threads = 4;
        cfg.rc_end_usage = VPX_CBR; // realtime rate control.
        cfg.rc_target_bitrate = self.target_kbps;
        // Small coder buffer keeps CBR latency low (values are in ms).
        cfg.rc_buf_sz = 1000;
        cfg.rc_buf_initial_sz = 500;
        cfg.rc_buf_optimal_sz = 600;
        // Drop-frame throttling would silently starve a stream on
        // congestion; the app-level pacer/congestion budget owns that.
        cfg.rc_dropframe_thresh = 0;

        // Init the context through a zeroed pointer, then move it into
        // `VpxCtx`. The ctx struct holds only a heap pointer to internal
        // state, so the move is sound (mirrors how encoders are handed
        // around by value).
        let mut ctx_uninit = std::mem::MaybeUninit::<vpx_codec_ctx_t>::zeroed();
        // SAFETY: `ctx` is zeroed (init_ver expects `priv == NULL`) and
        // initialized by `vpx_codec_enc_init_ver` with a valid iface, our
        // fully-populated cfg, and the ABI version the bindings target.
        let res = unsafe {
            vpx_codec_enc_init_ver(
                ctx_uninit.as_mut_ptr(),
                iface,
                &raw const cfg,
                0,
                c_int::try_from(VPX_ENCODER_ABI_VERSION).unwrap_or(0),
            )
        };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }
        // SAFETY: `init_ver` returned VPX_CODEC_OK — ctx is initialized.
        let ctx = unsafe { ctx_uninit.assume_init() };
        let mut inner = VpxCtx { ctx, cfg };

        // Realtime speed settings on the live context.
        let cpu_used = match self.codec {
            Codec::Vp8 => CPU_USED_VP8,
            Codec::Vp9 => CPU_USED_VP9,
            Codec::H264 => unreachable!("iface() already rejected H.264"),
        };
        // SAFETY: `inner.ctx` is an initialized encoder; `vpx_codec_control_`
        // is variadic and these control ids each take one `c_int` argument.
        let res =
            unsafe { vpx_codec_control_(&raw mut inner.ctx, VP8E_SET_CPUUSED as c_int, cpu_used) };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }
        // Static-threshold 0 (VP8) / row multithreading (VP9) — realtime
        // defaults libvpx's own vpxenc uses.
        let res = match self.codec {
            // SAFETY: `inner.ctx` is an initialized encoder; this control id
            // takes one `c_int` argument.
            Codec::Vp8 => unsafe {
                vpx_codec_control_(&raw mut inner.ctx, VP8E_SET_STATIC_THRESHOLD as c_int, 0)
            },
            // SAFETY: `inner.ctx` is an initialized encoder; this control id
            // takes one `c_int` argument.
            Codec::Vp9 => unsafe {
                vpx_codec_control_(&raw mut inner.ctx, VP9E_SET_ROW_MT as c_int, 1)
            },
            Codec::H264 => unreachable!("iface() already rejected H.264"),
        };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }

        self.inner = Some(inner);
        self.enc_dims = (width, height);
        self.next_pts = 0;
        self.needs_rebuild = false;
        tracing::debug!(
            target: "rekindle_video_libvpx",
            codec = self.codec.wire_str(),
            width,
            height,
            fps = self.fps,
            kbps = self.target_kbps,
            "libvpx realtime CBR encoder (re)built"
        );
        Ok(())
    }
}

impl VideoEncoder for LibvpxEncoder {
    fn configure(&mut self, cfg: &EncoderConstraints) -> Result<(), VideoError> {
        // Validate the codec eagerly so an unsupported pick fails now.
        Self::iface(cfg.codec)?;
        self.codec = cfg.codec;
        self.fps = cfg.max_fps.clamp(FPS_MIN, FPS_MAX);
        self.constraints = Some(cfg.clone());
        self.needs_rebuild = true;
        Ok(())
    }

    fn encode(&mut self, frame: RawFrame) -> Result<Option<EncodedVideoFrame>, VideoError> {
        if self.constraints.is_none() {
            return Err(VideoError::EncoderNotConfigured);
        }
        // Validates dimensions (even, non-zero) + plane sizes, then
        // flattens Y‖U‖V into the contiguous buffer libvpx wraps.
        let buf = frame.to_i420_contiguous()?;
        let dims = (frame.width, frame.height);

        if self.inner.is_none() || self.needs_rebuild || self.enc_dims != dims {
            self.rebuild(frame.width, frame.height)?;
        }

        let pts = self.next_pts;
        self.next_pts += 1;
        let flags: vpx_sys::vpx_enc_frame_flags_t = if self.force_kf_next {
            VPX_EFLAG_FORCE_KF.into()
        } else {
            0
        };
        self.force_kf_next = false;

        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| VideoError::Encode("encoder missing after rebuild".to_string()))?;

        // Wrap the borrowed I420 buffer as a vpx_image (no copy) and encode.
        let mut image_uninit = std::mem::MaybeUninit::<vpx_image_t>::zeroed();
        // SAFETY: `img_wrap` populates the image header through the ptr,
        // reading the dimensions and storing the (valid, live) `buf`
        // pointer; `buf` outlives the `vpx_codec_encode` call below.
        let wrapped = unsafe {
            vpx_img_wrap(
                image_uninit.as_mut_ptr(),
                VPX_IMG_FMT_I420,
                frame.width,
                frame.height,
                1,
                buf.as_ptr().cast_mut(),
            )
        };
        if wrapped.is_null() {
            return Err(VideoError::Encode("vpx_img_wrap returned null".to_string()));
        }
        // SAFETY: `img_wrap` returned non-null, so the image header is
        // initialized.
        let image = unsafe { image_uninit.assume_init() };

        // SAFETY: `inner.ctx` is initialized; `image` wraps the live `buf`;
        // realtime deadline, duration of one timebase tick.
        let res = unsafe {
            vpx_codec_encode(
                &raw mut inner.ctx,
                &raw const image,
                pts,
                1,
                flags,
                VPX_DL_REALTIME.into(),
            )
        };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }

        // Drain compressed output. A single-layer realtime encode yields at
        // most one frame packet per input; keep the first.
        let mut iter: vpx_codec_iter_t = ptr::null();
        let mut out: Option<EncodedVideoFrame> = None;
        loop {
            // SAFETY: `inner.ctx` is initialized; `iter` starts null and is
            // advanced by libvpx. The returned pointer is owned by libvpx
            // and valid until the next `vpx_codec_encode`; we copy out of it.
            let pkt = unsafe { vpx_codec_get_cx_data(&raw mut inner.ctx, &raw mut iter) };
            if pkt.is_null() {
                break;
            }
            // SAFETY: `pkt` is a valid non-null packet pointer from libvpx.
            let kind = unsafe { (*pkt).kind };
            if kind == VPX_CODEC_CX_FRAME_PKT && out.is_none() {
                // SAFETY: `kind == FRAME_PKT` means the union's `frame`
                // member is active; `buf`/`sz` describe a valid byte range.
                let payload = unsafe {
                    let f = &(*pkt).data.frame;
                    let keyframe = (f.flags & VPX_FRAME_IS_KEY) != 0;
                    let data = std::slice::from_raw_parts(f.buf.cast::<u8>(), f.sz);
                    (data.to_vec(), keyframe)
                };
                out = Some(EncodedVideoFrame {
                    payload: payload.0,
                    keyframe: payload.1,
                    timestamp_ms: frame.timestamp_ms,
                });
            }
        }
        Ok(out)
    }

    fn force_keyframe(&mut self) {
        // Applied on the next encode via VPX_EFLAG_FORCE_KF — no rebuild,
        // no lost inter-frame reference state.
        self.force_kf_next = true;
    }

    fn set_bitrate(&mut self, kbps: u32) {
        if kbps == self.target_kbps {
            return;
        }
        self.target_kbps = kbps;
        // Apply live to the running context (CBR retarget, no keyframe). If
        // there is no context yet, the next rebuild picks up target_kbps.
        if let Some(inner) = self.inner.as_mut() {
            inner.cfg.rc_target_bitrate = kbps;
            // SAFETY: `inner.ctx` is initialized and `inner.cfg` is the
            // config it was built from with only the bitrate changed —
            // exactly what `vpx_codec_enc_config_set` accepts live.
            let res = unsafe { vpx_codec_enc_config_set(&raw mut inner.ctx, &raw const inner.cfg) };
            if res != vpx_codec_err_t::VPX_CODEC_OK {
                // Non-fatal: log and let the next rebuild apply it. Pull the
                // detail string while the context is valid.
                // SAFETY: `inner.ctx` is a valid initialized context;
                // `vpx_codec_error` returns a static NUL-terminated string.
                let detail = unsafe {
                    let p = vpx_codec_error(&raw const inner.ctx);
                    if p.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(p).to_string_lossy().into_owned()
                    }
                };
                tracing::warn!(
                    target: "rekindle_video_libvpx",
                    kbps,
                    error = %format!("{res:?}"),
                    detail = %detail,
                    "live bitrate reconfigure failed; will apply on next rebuild"
                );
                self.needs_rebuild = true;
            }
        }
    }

    fn codec(&self) -> Codec {
        self.codec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::video::ScalabilityMode;

    fn constraints(codec: Codec, w: u32, h: u32) -> EncoderConstraints {
        EncoderConstraints {
            codec,
            max_width: w,
            max_height: h,
            max_fps: 15,
            scalability_mode: ScalabilityMode::Flat,
        }
    }

    /// A synthetic I420 frame with a gradient luma plane (real content, so
    /// rate control has something to compress).
    fn synthetic_frame(w: u32, h: u32, ts: u64) -> RawFrame {
        let mut y = vec![0u8; RawFrame::expected_y_len(w, h)];
        for (i, px) in y.iter_mut().enumerate() {
            *px = u8::try_from(i % 256).unwrap_or(0);
        }
        RawFrame {
            width: w,
            height: h,
            y,
            u: vec![0x60; RawFrame::expected_chroma_len(w, h)],
            v: vec![0xA0; RawFrame::expected_chroma_len(w, h)],
            timestamp_ms: ts,
        }
    }

    #[test]
    fn encodes_synthetic_vp9_keyframe() {
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        enc.configure(&constraints(Codec::Vp9, 320, 240)).unwrap();
        let out = enc
            .encode(synthetic_frame(320, 240, 0))
            .unwrap()
            .expect("realtime VP9 must emit the first frame (lag=0)");
        assert!(
            !out.payload.is_empty(),
            "encoded VP9 payload must be non-empty"
        );
        assert!(out.keyframe, "first frame must be a keyframe");
        assert_eq!(out.timestamp_ms, 0);
        assert_eq!(enc.codec(), Codec::Vp9);
        // Observed on macOS + libvpx 1.15.2: ~3.5 KB for this 320×240 frame.
        assert!(
            out.payload.len() > 100,
            "keyframe unexpectedly tiny: {} bytes",
            out.payload.len()
        );
    }

    #[test]
    fn encodes_synthetic_vp8_keyframe() {
        let mut enc = LibvpxEncoder::new(Codec::Vp8).unwrap();
        enc.configure(&constraints(Codec::Vp8, 320, 240)).unwrap();
        let out = enc
            .encode(synthetic_frame(320, 240, 0))
            .unwrap()
            .expect("realtime VP8 must emit the first frame");
        assert!(
            !out.payload.is_empty(),
            "encoded VP8 payload must be non-empty"
        );
        assert!(out.keyframe, "first frame must be a keyframe");
        assert_eq!(enc.codec(), Codec::Vp8);
        // Observed on macOS + libvpx 1.15.2: ~1.5 KB for this 320×240 frame.
        assert!(
            out.payload.len() > 100,
            "keyframe tiny: {} bytes",
            out.payload.len()
        );
    }

    #[test]
    fn force_keyframe_makes_next_frame_a_keyframe() {
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        enc.configure(&constraints(Codec::Vp9, 160, 120)).unwrap();
        // First frame is a keyframe; feed a couple inter frames.
        let _ = enc.encode(synthetic_frame(160, 120, 0)).unwrap();
        let _ = enc.encode(synthetic_frame(160, 120, 66)).unwrap();
        enc.force_keyframe();
        let kf = enc
            .encode(synthetic_frame(160, 120, 132))
            .unwrap()
            .expect("post-force frame produces output");
        assert!(kf.keyframe, "frame after force_keyframe must be a keyframe");
    }

    #[test]
    fn set_bitrate_live_does_not_error() {
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        enc.configure(&constraints(Codec::Vp9, 160, 120)).unwrap();
        let first = enc.encode(synthetic_frame(160, 120, 0)).unwrap();
        assert!(first.is_some());
        // Live retarget (no keyframe, no rebuild) then keep encoding.
        enc.set_bitrate(120);
        let next = enc.encode(synthetic_frame(160, 120, 66)).unwrap();
        // May be None if the RC dropped it, but must not error and must not
        // force a keyframe.
        if let Some(f) = next {
            assert!(!f.keyframe, "a bitrate change must not force a keyframe");
        }
    }

    #[test]
    fn encode_before_configure_errors() {
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        assert!(matches!(
            enc.encode(synthetic_frame(64, 64, 0)),
            Err(VideoError::EncoderNotConfigured)
        ));
    }

    #[test]
    fn h264_is_unsupported() {
        // libvpx has no H.264 encoder — rejected at construction and at
        // configure.
        assert!(matches!(
            LibvpxEncoder::new(Codec::H264),
            Err(VideoError::Unsupported(_))
        ));
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        assert!(matches!(
            enc.configure(&constraints(Codec::H264, 320, 240)),
            Err(VideoError::Unsupported(_))
        ));
    }

    #[test]
    fn odd_dimensions_rejected_cleanly() {
        let mut enc = LibvpxEncoder::new(Codec::Vp9).unwrap();
        enc.configure(&constraints(Codec::Vp9, 320, 240)).unwrap();
        let mut f = synthetic_frame(320, 240, 0);
        f.width = 321;
        f.y = vec![0; 321 * 240];
        f.u = vec![0; RawFrame::expected_chroma_len(321, 240)];
        f.v = vec![0; RawFrame::expected_chroma_len(321, 240)];
        assert!(matches!(enc.encode(f), Err(VideoError::InvalidInput(_))));
    }
}
