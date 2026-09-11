//! Step 2 — the `VideoEncoder` trait the architecture has long claimed
//! exists (`lib.rs:3` mentions a "`VideoCodec` trait" that had no
//! definition anywhere in the workspace; plan `video-media-engine.md`
//! §"Step 2 — extract the encoder behind a trait").
//!
//! This module is **pure**: it defines the raw-frame input contract, the
//! encoded-frame output, and the encoder trait — no codec FFI, no C
//! dependency. `rekindle-video` stays Tier-7 pure logic. Concrete
//! implementations live in sibling crates:
//!
//! | impl        | crate                    | dependency          |
//! |-------------|--------------------------|---------------------|
//! | libvpx      | `rekindle-video-libvpx`  | libvpx (feature-gated) |
//! | GStreamer   | `rekindle-video-capture` | GStreamer (Linux)   |
//! | WebCodecs   | webview                  | browser (fallback)  |
//!
//! `set_bitrate` and `force_keyframe` are the whole point of the trait:
//! they are what the `VideoBitrateTarget` and `VideoKeyframeRequest`
//! control messages currently cross the IPC boundary to do. Once the
//! encoder is backend-side (`rekindle-node`), those become internal
//! calls.

use crate::error::VideoError;
use rekindle_types::video::Codec;

use crate::EncoderConstraints;

/// A raw, uncompressed video frame in **I420** (a.k.a. YUV 4:2:0 planar,
/// `VPX_IMG_FMT_I420`) — the format every software VP8/VP9 encoder
/// consumes and the format the GStreamer capture pipeline already emits
/// (`pipeline/mod.rs` negotiates `video/x-raw,format=I420`).
///
/// # Pixel format (I420)
///
/// Three **tightly packed** planes, in this order, one contiguous
/// logical image with no inter-row padding:
///
/// - **Y** (luma): `width × height` bytes, one byte per pixel, row
///   stride == `width`.
/// - **U** (Cb, chroma): `(width / 2) × (height / 2)` bytes, row stride
///   == `width / 2`.
/// - **V** (Cr, chroma): `(width / 2) × (height / 2)` bytes, row stride
///   == `width / 2`.
///
/// Total length is therefore `width * height * 3 / 2`. Chroma is
/// subsampled 2×2, so **both `width` and `height` must be even** — the
/// libvpx encoder rejects odd dimensions. Planes carry no stride padding:
/// a capture source that hands back padded strides must repack before
/// building a `RawFrame` (scaling / stride normalisation is the
/// pipeline's job, exactly as the GStreamer `videoscale` branch does it
/// before the encoder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    /// Frame width in pixels. Must be even and non-zero.
    pub width: u32,
    /// Frame height in pixels. Must be even and non-zero.
    pub height: u32,
    /// Luma plane, `width * height` bytes, tightly packed.
    pub y: Vec<u8>,
    /// Cb chroma plane, `(width / 2) * (height / 2)` bytes, tightly packed.
    pub u: Vec<u8>,
    /// Cr chroma plane, `(width / 2) * (height / 2)` bytes, tightly packed.
    pub v: Vec<u8>,
    /// Presentation timestamp in milliseconds (monotonic, wall-clock
    /// origin is the caller's). The encoder stamps this straight onto the
    /// encoded output so the wire timestamp survives the encode.
    pub timestamp_ms: u64,
}

impl RawFrame {
    /// Expected `y` plane length for a `width × height` I420 frame.
    #[must_use]
    pub fn expected_y_len(width: u32, height: u32) -> usize {
        width as usize * height as usize
    }

    /// Expected `u` / `v` plane length for a `width × height` I420 frame
    /// (chroma subsampled 2×2, rounding the odd tail up the same way
    /// libvpx does).
    #[must_use]
    pub fn expected_chroma_len(width: u32, height: u32) -> usize {
        let cw = width.div_ceil(2) as usize;
        let ch = height.div_ceil(2) as usize;
        cw * ch
    }

    /// Validate that the planes match the declared `width`/`height` and
    /// that the dimensions are the even, non-zero shape a VP8/VP9 encoder
    /// requires. Returns a descriptive [`VideoError::InvalidInput`] rather
    /// than letting the encoder panic on a size mismatch.
    pub fn validate(&self) -> Result<(), VideoError> {
        if self.width == 0 || self.height == 0 {
            return Err(VideoError::InvalidInput(format!(
                "frame dimensions must be non-zero (got {}×{})",
                self.width, self.height
            )));
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            return Err(VideoError::InvalidInput(format!(
                "I420 requires even dimensions (got {}×{})",
                self.width, self.height
            )));
        }
        let want_y = Self::expected_y_len(self.width, self.height);
        let want_c = Self::expected_chroma_len(self.width, self.height);
        if self.y.len() != want_y {
            return Err(VideoError::InvalidInput(format!(
                "Y plane length {} != expected {want_y} for {}×{}",
                self.y.len(),
                self.width,
                self.height
            )));
        }
        if self.u.len() != want_c || self.v.len() != want_c {
            return Err(VideoError::InvalidInput(format!(
                "chroma plane lengths (u={}, v={}) != expected {want_c} for {}×{}",
                self.u.len(),
                self.v.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }

    /// Flatten the three planes into the single contiguous I420 buffer a
    /// software encoder wraps (`Y ‖ U ‖ V`). Validates first.
    pub fn to_i420_contiguous(&self) -> Result<Vec<u8>, VideoError> {
        self.validate()?;
        let mut buf = Vec::with_capacity(self.y.len() + self.u.len() + self.v.len());
        buf.extend_from_slice(&self.y);
        buf.extend_from_slice(&self.u);
        buf.extend_from_slice(&self.v);
        Ok(buf)
    }
}

/// One compressed output frame produced by a [`VideoEncoder`]. Distinct
/// from `rekindle_video_capture::pipeline::EncodedFrame` (which lacks a
/// timestamp — the native pump stamps wall-clock ms downstream); this is
/// the pure-home canonical output the trait promises, and it carries the
/// presentation timestamp through from the source [`RawFrame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedVideoFrame {
    /// The compressed bitstream for this frame (a single VP8/VP9 frame).
    pub payload: Vec<u8>,
    /// Whether this is a keyframe (intra / IDR) — a decoder can start or
    /// recover from here without prior frames.
    pub keyframe: bool,
    /// Presentation timestamp in milliseconds, copied from the source
    /// [`RawFrame::timestamp_ms`].
    pub timestamp_ms: u64,
}

/// A realtime video encoder. The trait is `Send` so the encoder can live
/// on a dedicated encode thread (the same posture `rekindle-voice` uses
/// for `cpal` streams) and be handed frames over a channel.
///
/// Lifecycle: [`configure`](VideoEncoder::configure) once (or again to
/// re-negotiate), then [`encode`](VideoEncoder::encode) per source frame.
/// [`set_bitrate`](VideoEncoder::set_bitrate) and
/// [`force_keyframe`](VideoEncoder::force_keyframe) are the congestion-
/// control and packet-loss-recovery levers, applied to the next encode.
pub trait VideoEncoder: Send {
    /// Configure (or reconfigure) the encoder for a negotiated session.
    /// Takes the room-wide [`EncoderConstraints`] the policy negotiator
    /// produced (codec, resolution ceiling, framerate, scalability mode).
    fn configure(&mut self, cfg: &EncoderConstraints) -> Result<(), VideoError>;

    /// Encode one raw frame. Returns `Ok(Some(frame))` when the encoder
    /// produced compressed output for this input, `Ok(None)` when it
    /// produced none (an encoder may buffer), or `Err` on failure.
    fn encode(&mut self, frame: RawFrame) -> Result<Option<EncodedVideoFrame>, VideoError>;

    /// Request that the **next** encoded frame be a keyframe — the
    /// FIR / PLI analog (`VideoKeyframeRequest`). Cheap and idempotent;
    /// callers throttle (the existing keyframe floor).
    fn force_keyframe(&mut self);

    /// Set the target bitrate in kilobits per second — the TMMBR analog
    /// (`VideoBitrateTarget`), driven by the receiver bandwidth feedback
    /// loop. Applies to subsequent frames.
    fn set_bitrate(&mut self, kbps: u32);

    /// The codec this encoder currently emits.
    fn codec(&self) -> Codec;
}

/// A realtime video decoder — the receive-side counterpart of
/// [`VideoEncoder`]. Like the encoder it is `Send`, so it can live on a
/// dedicated decode thread (one per remote stream) and be fed frames over
/// a channel, matching the `rekindle-voice` per-participant posture.
///
/// Unlike the encoder there is **no `configure`**: the codec is fixed at
/// construction (a VP8 stream needs a VP8 decoder, a VP9 stream a VP9 one —
/// libvpx has no "auto" codec), and a VP8/VP9 bitstream is otherwise
/// self-describing (frame dimensions, keyframe flag, and scaling all ride
/// in the bitstream headers), so there is nothing left to negotiate on the
/// receive side. Feed [`decode`](VideoDecoder::decode) each
/// [`EncodedVideoFrame`] in arrival order; it returns the reconstructed
/// [`RawFrame`] (I420) or `Ok(None)` when this input yielded no displayable
/// frame yet.
pub trait VideoDecoder: Send {
    /// Decode one compressed frame. Returns `Ok(Some(frame))` with the
    /// reconstructed I420 [`RawFrame`] when this input produced a
    /// displayable frame, `Ok(None)` when it produced none (e.g. an empty
    /// payload, or a hidden/alt-ref frame that carries no visible output),
    /// or `Err` on a decode failure.
    ///
    /// A decoder cannot produce output before it has seen a keyframe; feed
    /// frames in order and expect `Ok(None)` until the first keyframe
    /// arrives.
    fn decode(&mut self, frame: &EncodedVideoFrame) -> Result<Option<RawFrame>, VideoError>;

    /// The codec this decoder consumes (fixed at construction).
    fn codec(&self) -> Codec;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame(width: u32, height: u32, ts: u64) -> RawFrame {
        RawFrame {
            width,
            height,
            y: vec![0x80; RawFrame::expected_y_len(width, height)],
            u: vec![0x80; RawFrame::expected_chroma_len(width, height)],
            v: vec![0x80; RawFrame::expected_chroma_len(width, height)],
            timestamp_ms: ts,
        }
    }

    #[test]
    fn valid_frame_flattens_to_1_5x() {
        let f = solid_frame(16, 16, 42);
        f.validate().unwrap();
        let buf = f.to_i420_contiguous().unwrap();
        // 16*16 Y + 8*8 U + 8*8 V = 256 + 64 + 64 = 384 = 16*16*3/2.
        assert_eq!(buf.len(), 384);
    }

    #[test]
    fn odd_dimensions_rejected() {
        let mut f = solid_frame(16, 16, 0);
        f.width = 15;
        // Resize Y so only the parity check trips, not the length check.
        f.y = vec![0; 15 * 16];
        let err = f.validate().unwrap_err();
        assert!(matches!(err, VideoError::InvalidInput(_)));
    }

    #[test]
    fn zero_dimensions_rejected() {
        let f = solid_frame(0, 0, 0);
        assert!(matches!(f.validate(), Err(VideoError::InvalidInput(_))));
    }

    #[test]
    fn plane_size_mismatch_rejected() {
        let mut f = solid_frame(16, 16, 0);
        f.y.pop();
        assert!(matches!(f.validate(), Err(VideoError::InvalidInput(_))));
    }

    #[test]
    fn expected_lengths_match_i420() {
        assert_eq!(RawFrame::expected_y_len(854, 480), 854 * 480);
        assert_eq!(RawFrame::expected_chroma_len(854, 480), 427 * 240);
    }
}
