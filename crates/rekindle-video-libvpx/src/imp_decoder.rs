//! Feature-ON path (`feature = "libvpx"`) — the VP8/VP9 realtime decoder,
//! the receive-side counterpart of [`imp::LibvpxEncoder`](crate::imp).
//!
//! It lives in its own module (rather than swelling `imp.rs` past the
//! 600-line ceiling) and, like the encoder, carries the C dependency and
//! is the only other place `unsafe` is used. Bound directly to libvpx
//! through `env-libvpx-sys` (`vpx_sys`): `vpx_codec_dec_init_ver` +
//! `vpx_codec_decode` + `vpx_codec_get_frame`, with a matching
//! `vpx_codec_destroy` in `Drop`.
//!
//! # Stride is NOT width
//!
//! libvpx hands back a `vpx_image_t` whose planes are **row-padded**:
//! `img.stride[i]` is generally larger than the plane's pixel width (rows
//! are aligned for SIMD). A `RawFrame` is tightly packed, so the copy runs
//! **row by row per plane**, taking exactly `row_bytes` from the front of
//! each strided source row — never `stride * rows` as one blit, which
//! would fold the padding into the output. Chroma planes are
//! `ceil(w/2) × ceil(h/2)` (I420 4:2:0), matching
//! [`RawFrame::expected_chroma_len`].

use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::ptr;

use rekindle_types::video::Codec;
use rekindle_video::codec::{EncodedVideoFrame, RawFrame, VideoDecoder};
use rekindle_video::error::VideoError;

use vpx_sys::{
    vpx_codec_ctx_t, vpx_codec_dec_cfg_t, vpx_codec_dec_init_ver, vpx_codec_decode,
    vpx_codec_destroy, vpx_codec_err_t, vpx_codec_get_frame, vpx_codec_iface, vpx_codec_iter_t,
    vpx_codec_vp8_dx, vpx_codec_vp9_dx, vpx_image_t, vpx_img_fmt::VPX_IMG_FMT_I420,
    VPX_DECODER_ABI_VERSION, VPX_PLANE_U, VPX_PLANE_V, VPX_PLANE_Y,
};

/// Decoder thread count. VP9 tile / VP8 frame threading; a small fixed
/// pool matches the encoder's `g_threads = 4`.
const DEC_THREADS: u32 = 4;

/// Map a non-OK libvpx status to a [`VideoError::Decode`]. The
/// `vpx_codec_err_t` variant name (e.g. `VPX_CODEC_UNSUP_BITSTREAM`) is
/// self-describing.
fn err(res: vpx_codec_err_t) -> VideoError {
    VideoError::Decode(format!("libvpx: {res:?}"))
}

/// An initialized libvpx decoder context. Its `Drop` runs the matching
/// `vpx_codec_destroy`.
struct VpxDecCtx {
    ctx: vpx_codec_ctx_t,
}

impl Drop for VpxDecCtx {
    fn drop(&mut self) {
        // SAFETY: `ctx` was initialized by `vpx_codec_dec_init_ver` (a
        // `VpxDecCtx` is only ever constructed after a successful init in
        // `LibvpxDecoder::new`), and `vpx_codec_destroy` is its matching
        // teardown.
        unsafe {
            vpx_codec_destroy(&raw mut self.ctx);
        }
    }
}

/// libvpx VP8/VP9 realtime decoder implementing
/// [`rekindle_video::codec::VideoDecoder`].
pub struct LibvpxDecoder {
    codec: Codec,
    /// The live decoder context. Initialized once in [`Self::new`] (the
    /// codec is fixed at construction, and a VP8/VP9 bitstream is
    /// self-describing, so there is no lazy reconfigure).
    inner: VpxDecCtx,
}

// SAFETY: `LibvpxDecoder` owns its libvpx decoder context exclusively and
// only ever touches it through `&mut self`, so it is never accessed
// concurrently. libvpx decoder contexts hold no caller-thread-local state
// and are sound to move between threads provided calls stay serialized —
// the same rationale the encoder (`imp.rs`) uses. That makes the raw
// pointers inside `vpx_codec_ctx_t` (which mark it `!Send`) safe to send.
unsafe impl Send for LibvpxDecoder {}

impl LibvpxDecoder {
    /// Create a decoder for `codec` (VP8 or VP9). The codec is fixed for
    /// the life of the decoder (it must match the incoming stream). H.264
    /// is not a libvpx codec and is rejected with
    /// [`VideoError::Unsupported`].
    pub fn new(codec: Codec) -> Result<Self, VideoError> {
        let iface = Self::iface(codec)?;

        // `w`/`h` = 0 lets libvpx size buffers from the stream headers on
        // the first frame (the bitstream is self-describing). `threads`
        // enables multi-threaded decode.
        let cfg = vpx_codec_dec_cfg_t {
            threads: DEC_THREADS,
            w: 0,
            h: 0,
        };

        let mut ctx_uninit = MaybeUninit::<vpx_codec_ctx_t>::zeroed();
        // SAFETY: `ctx` is zeroed (init_ver expects `priv == NULL`); `iface`
        // is a valid non-null decoder interface; `cfg` is fully populated;
        // and we pass the ABI version the bindings target. On OK, the ctx is
        // initialized.
        let res = unsafe {
            vpx_codec_dec_init_ver(
                ctx_uninit.as_mut_ptr(),
                iface,
                &raw const cfg,
                0,
                c_int::try_from(VPX_DECODER_ABI_VERSION).unwrap_or(0),
            )
        };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }
        // SAFETY: `dec_init_ver` returned VPX_CODEC_OK — ctx is initialized.
        let ctx = unsafe { ctx_uninit.assume_init() };

        tracing::debug!(
            target: "rekindle_video_libvpx",
            codec = codec.wire_str(),
            threads = DEC_THREADS,
            "libvpx realtime decoder initialized"
        );
        Ok(Self {
            codec,
            inner: VpxDecCtx { ctx },
        })
    }

    /// The libvpx *decoder* interface for our codec. H.264 has no libvpx
    /// decoder.
    fn iface(codec: Codec) -> Result<*const vpx_codec_iface, VideoError> {
        let iface = match codec {
            // SAFETY: returns a pointer to libvpx's static VP8 decoder
            // interface singleton; no arguments, no invariants to uphold.
            Codec::Vp8 => unsafe { vpx_codec_vp8_dx() },
            // SAFETY: returns a pointer to libvpx's static VP9 decoder
            // interface singleton; no arguments, no invariants to uphold.
            Codec::Vp9 => unsafe { vpx_codec_vp9_dx() },
            Codec::H264 => {
                return Err(VideoError::Unsupported(
                    "libvpx decodes VP8/VP9 only — H.264 needs a different engine \
                     (WebCodecs/VideoToolbox/GStreamer)"
                        .to_string(),
                ))
            }
        };
        if iface.is_null() {
            return Err(VideoError::Decode(
                "libvpx returned a null codec interface".to_string(),
            ));
        }
        Ok(iface)
    }
}

impl VideoDecoder for LibvpxDecoder {
    fn decode(&mut self, frame: &EncodedVideoFrame) -> Result<Option<RawFrame>, VideoError> {
        // An empty payload carries no coded frame — nothing to feed libvpx.
        if frame.payload.is_empty() {
            return Ok(None);
        }
        let data_sz = u32::try_from(frame.payload.len())
            .map_err(|_| VideoError::Decode("frame payload length exceeds u32::MAX".to_string()))?;

        // SAFETY: `inner.ctx` was initialized in `new`. `payload` is a live,
        // contiguous byte range of `data_sz` bytes valid for the whole call;
        // `user_priv` is unused (null) and `deadline` 0 decodes eagerly.
        let res = unsafe {
            vpx_codec_decode(
                &raw mut self.inner.ctx,
                frame.payload.as_ptr(),
                data_sz,
                ptr::null_mut(),
                0,
            )
        };
        if res != vpx_codec_err_t::VPX_CODEC_OK {
            return Err(err(res));
        }

        // Pull the first displayable frame. A single-layer realtime stream
        // yields at most one visible frame per input packet; hidden/alt-ref
        // frames are not returned by `get_frame` at all.
        let mut iter: vpx_codec_iter_t = ptr::null();
        // SAFETY: `inner.ctx` is initialized; `iter` starts null and is
        // advanced by libvpx. The returned image pointer is owned by libvpx
        // and valid until the next `vpx_codec_decode`; we copy out of it now.
        let img = unsafe { vpx_codec_get_frame(&raw mut self.inner.ctx, &raw mut iter) };
        if img.is_null() {
            // Valid: an inter frame fed before the first keyframe, or a
            // packet that produced no visible output yet.
            return Ok(None);
        }

        // SAFETY: `img` is a valid, non-null image just returned by
        // `get_frame`, live until the next decode call; its plane pointers
        // and strides describe the decoded I420 image.
        let raw = unsafe { image_to_raw_frame(img, frame.timestamp_ms) }?;
        Ok(Some(raw))
    }

    fn codec(&self) -> Codec {
        self.codec
    }
}

/// Copy a libvpx-owned `vpx_image_t` (expected I420) into an owned,
/// tightly-packed [`RawFrame`], honoring each plane's row **stride**.
///
/// # Safety
///
/// `img` must be a valid, non-null pointer to an initialized `vpx_image_t`
/// as returned by `vpx_codec_get_frame`, with plane pointers valid for
/// their strided extents and the image live for the duration of the call.
unsafe fn image_to_raw_frame(
    img: *const vpx_image_t,
    timestamp_ms: u64,
) -> Result<RawFrame, VideoError> {
    // SAFETY: by this fn's contract `img` is a valid, initialized image ptr,
    // so a shared reference to the header is sound for this call.
    let image = unsafe { &*img };

    if image.fmt != VPX_IMG_FMT_I420 {
        return Err(VideoError::Decode(format!(
            "decoder returned a non-I420 image (fmt={:?}); only I420 is supported",
            image.fmt
        )));
    }

    // Display dimensions (`d_w`/`d_h`) are the visible frame size the
    // encoder was fed; `w`/`h` may be padded up to a macroblock multiple.
    let width = image.d_w;
    let height = image.d_h;
    if width == 0 || height == 0 {
        return Err(VideoError::Decode(format!(
            "decoder returned a zero-sized image ({width}×{height})"
        )));
    }

    // I420 chroma is subsampled 2×2, rounding the odd tail up the same way
    // `RawFrame::expected_chroma_len` (and libvpx) do.
    let cw = (width as usize).div_ceil(2);
    let ch = (height as usize).div_ceil(2);

    let y = copy_plane(image, VPX_PLANE_Y as usize, width as usize, height as usize)?;
    let u = copy_plane(image, VPX_PLANE_U as usize, cw, ch)?;
    let v = copy_plane(image, VPX_PLANE_V as usize, cw, ch)?;

    Ok(RawFrame {
        width,
        height,
        y,
        u,
        v,
        timestamp_ms,
    })
}

/// Copy one plane out of a decoded image into a tightly-packed `Vec<u8>`,
/// taking `row_bytes` from the front of each of `rows` strided source rows.
/// This is where stride ≠ width is respected: `img.stride[plane]` is the
/// source row pitch, `row_bytes` is what actually belongs to the plane.
fn copy_plane(
    image: &vpx_image_t,
    plane: usize,
    row_bytes: usize,
    rows: usize,
) -> Result<Vec<u8>, VideoError> {
    let src = image.planes[plane];
    if src.is_null() {
        return Err(VideoError::Decode(format!(
            "decoder image plane {plane} pointer is null"
        )));
    }
    let stride = usize::try_from(image.stride[plane]).map_err(|_| {
        VideoError::Decode(format!(
            "decoder image plane {plane} has a negative stride ({})",
            image.stride[plane]
        ))
    })?;
    if stride < row_bytes {
        return Err(VideoError::Decode(format!(
            "decoder image plane {plane} stride {stride} < row width {row_bytes}"
        )));
    }

    let mut out = Vec::with_capacity(row_bytes * rows);
    for row in 0..rows {
        // SAFETY: `src` is a valid, non-null plane base pointer; libvpx
        // guarantees the plane holds at least `rows` rows of `stride` bytes,
        // so `src + row*stride` starts row `row` in-bounds and the
        // `row_bytes` (≤ `stride`, checked above) we read stay within it.
        let row_ptr = unsafe { src.add(row * stride) };
        // SAFETY: `row_ptr .. row_ptr + row_bytes` lies within the plane's
        // `row`-th row (same guarantee), and `u8` has no alignment or
        // initialization constraints beyond a live readable range.
        let row_slice = unsafe { std::slice::from_raw_parts(row_ptr, row_bytes) };
        out.extend_from_slice(row_slice);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibvpxEncoder;
    use rekindle_types::video::ScalabilityMode;
    use rekindle_video::codec::VideoEncoder;
    use rekindle_video::EncoderConstraints;

    fn constraints(codec: Codec, w: u32, h: u32) -> EncoderConstraints {
        EncoderConstraints {
            codec,
            max_width: w,
            max_height: h,
            max_fps: 15,
            scalability_mode: ScalabilityMode::Flat,
        }
    }

    /// A synthetic I420 frame with a gradient luma plane and constant chroma
    /// — real content, so the encoded frame is non-trivial and the decoded
    /// luma must carry the gradient back.
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

    /// Encode a synthetic frame then decode it back — proving the
    /// encode↔decode pair end to end for one codec. Not pixel equality
    /// (VP8/VP9 are lossy); we assert dimensions, plane sizes, that the
    /// decoded luma is non-trivial, and that the wire timestamp survives.
    fn round_trip(codec: Codec) -> (usize, RawFrame) {
        let (w, h) = (320u32, 240u32);

        let mut enc = LibvpxEncoder::new(codec).expect("encoder construction");
        enc.configure(&constraints(codec, w, h))
            .expect("encoder configure");
        let encoded = enc
            .encode(synthetic_frame(w, h, 7))
            .expect("encode must not error")
            .expect("realtime encoder emits the first frame (lag=0)");
        assert!(encoded.keyframe, "first encoded frame must be a keyframe");
        assert!(!encoded.payload.is_empty(), "encoded payload non-empty");

        let mut dec = LibvpxDecoder::new(codec).expect("decoder construction");
        assert_eq!(dec.codec(), codec);
        let decoded = dec
            .decode(&encoded)
            .expect("decode must not error")
            .expect("a keyframe must yield a displayable frame");

        // Right dimensions (from the bitstream, via d_w/d_h).
        assert_eq!(decoded.width, w, "decoded width");
        assert_eq!(decoded.height, h, "decoded height");
        // Plane sizes match tightly-packed I420 — proves stride was honored
        // (a naive stride*rows blit would over-copy the SIMD row padding).
        assert_eq!(
            decoded.y.len(),
            RawFrame::expected_y_len(w, h),
            "Y plane must be width*height, tightly packed"
        );
        assert_eq!(
            decoded.u.len(),
            RawFrame::expected_chroma_len(w, h),
            "U plane must be ceil(w/2)*ceil(h/2)"
        );
        assert_eq!(
            decoded.v.len(),
            RawFrame::expected_chroma_len(w, h),
            "V plane must be ceil(w/2)*ceil(h/2)"
        );
        // The reconstructed frame is itself a valid RawFrame.
        decoded.validate().expect("decoded frame validates");
        // The gradient luma survived (not a blank/zero plane).
        assert!(
            decoded.y.iter().any(|&b| b != 0),
            "decoded luma must carry the gradient"
        );
        // Wire timestamp passes straight through the decoder.
        assert_eq!(decoded.timestamp_ms, 7, "timestamp passthrough");

        (encoded.payload.len(), decoded)
    }

    #[test]
    fn round_trip_vp9() {
        let (encoded_bytes, decoded) = round_trip(Codec::Vp9);
        // Observed on macOS + libvpx 1.15.2: 320×240 keyframe ~3–4 KB.
        assert!(
            encoded_bytes > 100,
            "VP9 keyframe unexpectedly tiny: {encoded_bytes} bytes"
        );
        assert_eq!(decoded.y.len(), 320 * 240);
        assert_eq!(decoded.u.len(), 160 * 120);
    }

    #[test]
    fn round_trip_vp8() {
        let (encoded_bytes, decoded) = round_trip(Codec::Vp8);
        // Observed on macOS + libvpx 1.15.2: 320×240 keyframe ~1–2 KB.
        assert!(
            encoded_bytes > 100,
            "VP8 keyframe unexpectedly tiny: {encoded_bytes} bytes"
        );
        assert_eq!(decoded.y.len(), 320 * 240);
        assert_eq!(decoded.v.len(), 160 * 120);
    }

    #[test]
    fn empty_payload_yields_none() {
        let mut dec = LibvpxDecoder::new(Codec::Vp9).unwrap();
        let empty = EncodedVideoFrame {
            payload: Vec::new(),
            keyframe: false,
            timestamp_ms: 0,
        };
        assert!(matches!(dec.decode(&empty), Ok(None)));
    }

    #[test]
    fn h264_decoder_is_unsupported() {
        assert!(matches!(
            LibvpxDecoder::new(Codec::H264),
            Err(VideoError::Unsupported(_))
        ));
    }
}
