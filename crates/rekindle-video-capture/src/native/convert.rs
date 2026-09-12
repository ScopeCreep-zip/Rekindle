//! Camera pixel-format → I420 conversion.
//!
//! The macOS AVFoundation and Windows Media Foundation cameras nokhwa opens
//! deliver frames as **NV12** (bi-planar Y + interleaved Cb,Cr) or **YUYV**
//! (packed 4:2:2). Both are converted to the tightly-packed I420 the libvpx
//! encoder consumes: NV12 is a pure plane de-interleave (no colour maths,
//! exact), YUYV → I420 the vertical-chroma-decimating repack, delegated to
//! the SIMD `yuv` crate (`yuyv422_to_yuv420`). Any other source format is
//! rejected by the caller (`session::to_i420`) — an MJPEG-only camera is
//! not yet handled on the native path.
//!
//! Both converters assume the camera buffer is TIGHTLY packed (row stride
//! == width); nokhwa's platform bindings copy each frame into a tight
//! buffer, so this holds for the formats above. The `yuv` crate's output
//! planes are read back through their reported strides, so any padding it
//! introduces is repacked away here.

use rekindle_video::codec::RawFrame;

use crate::shared::CaptureError;

/// An owned I420 (YUV 4:2:0 planar) frame with tightly-packed planes — the
/// intermediate the convert and scale stages pass around before a
/// `RawFrame` is built for the encoder.
pub(crate) struct I420Buf {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl I420Buf {
    /// Consume into the encoder's `RawFrame`, stamping the presentation
    /// timestamp (the wire timestamp is applied downstream by the pump,
    /// exactly as the GStreamer path does).
    pub(crate) fn into_raw_frame(self, timestamp_ms: u64) -> RawFrame {
        RawFrame {
            width: self.width,
            height: self.height,
            y: self.y,
            u: self.u,
            v: self.v,
            timestamp_ms,
        }
    }
}

fn require_even(width: u32, height: u32) -> Result<(), CaptureError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(CaptureError::Pipeline(format!(
            "I420 needs even, non-zero dimensions (got {width}×{height})"
        )));
    }
    Ok(())
}

/// NV12 (Y plane, then interleaved Cb,Cr) → I420 (Y, then split U, V).
pub(crate) fn nv12_to_i420(bytes: &[u8], width: u32, height: u32) -> Result<I420Buf, CaptureError> {
    require_even(width, height)?;
    let w = width as usize;
    let h = height as usize;
    let y_len = w * h;
    let chroma = (w / 2) * (h / 2);
    let uv_len = chroma * 2;
    if bytes.len() < y_len + uv_len {
        return Err(CaptureError::Pipeline(format!(
            "NV12 buffer too small: {} < {} for {width}×{height}",
            bytes.len(),
            y_len + uv_len
        )));
    }
    let y_plane = bytes[..y_len].to_vec();
    let mut u_plane = Vec::with_capacity(chroma);
    let mut v_plane = Vec::with_capacity(chroma);
    for pair in bytes[y_len..y_len + uv_len].chunks_exact(2) {
        u_plane.push(pair[0]);
        v_plane.push(pair[1]);
    }
    Ok(I420Buf {
        width,
        height,
        y: y_plane,
        u: u_plane,
        v: v_plane,
    })
}

/// YUYV / YUY2 (packed 4:2:2) → I420, via the SIMD `yuv` crate.
pub(crate) fn yuyv_to_i420(bytes: &[u8], width: u32, height: u32) -> Result<I420Buf, CaptureError> {
    require_even(width, height)?;
    let need = width as usize * height as usize * 2;
    if bytes.len() < need {
        return Err(CaptureError::Pipeline(format!(
            "YUYV buffer too small: {} < {need} for {width}×{height}",
            bytes.len()
        )));
    }
    let packed = yuv::YuvPackedImage {
        yuy: &bytes[..need],
        yuy_stride: width * 2,
        width,
        height,
    };
    let mut planar =
        yuv::YuvPlanarImageMut::<u8>::alloc(width, height, yuv::YuvChromaSubsampling::Yuv420);
    yuv::yuyv422_to_yuv420(&mut planar, &packed)
        .map_err(|e| CaptureError::Pipeline(format!("YUYV→I420: {e:?}")))?;
    Ok(I420Buf {
        width,
        height,
        y: tight_plane(planar.y_plane.borrow(), planar.y_stride, width, height),
        u: tight_plane(
            planar.u_plane.borrow(),
            planar.u_stride,
            width / 2,
            height / 2,
        ),
        v: tight_plane(
            planar.v_plane.borrow(),
            planar.v_stride,
            width / 2,
            height / 2,
        ),
    })
}

/// Copy a strided plane into a tightly-packed `width × height` buffer.
fn tight_plane(buf: &[u8], stride: u32, width: u32, height: u32) -> Vec<u8> {
    let stride = stride as usize;
    let width = width as usize;
    let mut out = Vec::with_capacity(width * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        out.extend_from_slice(&buf[start..start + width]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_deinterleaves_to_i420_sizes() {
        let (w, h) = (4u32, 4u32);
        // Y plane: 16 bytes; UV plane: 2×2 = 4 interleaved (U,V) pairs.
        let mut bytes = vec![0u8; 16];
        for i in 0..4u8 {
            bytes.push(10 + i); // U
            bytes.push(20 + i); // V
        }
        let f = nv12_to_i420(&bytes, w, h).unwrap();
        assert_eq!((f.width, f.height), (4, 4));
        assert_eq!(f.y.len(), 16);
        assert_eq!(f.u.len(), 4);
        assert_eq!(f.v.len(), 4);
        assert_eq!(f.u, vec![10, 11, 12, 13]);
        assert_eq!(f.v, vec![20, 21, 22, 23]);
    }

    #[test]
    fn nv12_rejects_short_buffer() {
        assert!(nv12_to_i420(&[0u8; 4], 4, 4).is_err());
    }

    #[test]
    fn nv12_rejects_odd_dims() {
        assert!(nv12_to_i420(&[0u8; 256], 5, 4).is_err());
    }

    #[test]
    fn yuyv_converts_to_i420_sizes() {
        let (w, h) = (4u32, 4u32);
        // Packed YUYV is 2 bytes/pixel → 32 bytes.
        let bytes = vec![128u8; (w * h * 2) as usize];
        let f = yuyv_to_i420(&bytes, w, h).unwrap();
        assert_eq!((f.width, f.height), (4, 4));
        assert_eq!(f.y.len(), 16);
        assert_eq!(f.u.len(), 4);
        assert_eq!(f.v.len(), 4);
    }

    #[test]
    fn yuyv_rejects_short_buffer() {
        assert!(yuyv_to_i420(&[0u8; 8], 4, 4).is_err());
    }

    #[test]
    fn into_raw_frame_validates_and_stamps() {
        let bytes = vec![128u8; (4 * 4 * 2) as usize];
        let raw = yuyv_to_i420(&bytes, 4, 4).unwrap().into_raw_frame(7);
        raw.validate().unwrap();
        assert_eq!(raw.timestamp_ms, 7);
    }
}
