//! Local self-view: a small JPEG still per frame.
//!
//! The peer egress carries VP9; the local preview the webview paints to a
//! canvas is a codec-stateless JPEG (mirrors the GStreamer path's `jpegenc`
//! branch and `PreviewFrame`). The scaled thumbnail is converted I420 → RGB
//! (`yuv` crate) then JPEG-encoded (`image`, `jpeg` feature only).

use super::convert::I420Buf;
use crate::shared::CaptureError;

/// Encode a (small) I420 thumbnail as a JPEG for the self-view channel.
pub(crate) fn encode_preview_jpeg(thumb: &I420Buf, quality: u8) -> Result<Vec<u8>, CaptureError> {
    let (w, h) = (thumb.width, thumb.height);
    let planar = yuv::YuvPlanarImage {
        y_plane: &thumb.y,
        y_stride: w,
        u_plane: &thumb.u,
        u_stride: w / 2,
        v_plane: &thumb.v,
        v_stride: w / 2,
        width: w,
        height: h,
    };
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    yuv::yuv420_to_rgb(
        &planar,
        &mut rgb,
        w * 3,
        yuv::YuvRange::Limited,
        yuv::YuvStandardMatrix::Bt601,
    )
    .map_err(|e| CaptureError::Pipeline(format!("I420→RGB: {e:?}")))?;

    let mut jpeg = Vec::new();
    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality);
        encoder
            .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
            .map_err(|e| CaptureError::Pipeline(format!("JPEG encode: {e}")))?;
    }
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_produces_a_nonempty_jpeg() {
        let thumb = I420Buf {
            width: 8,
            height: 8,
            y: vec![120; 64],
            u: vec![100; 16],
            v: vec![140; 16],
        };
        let jpeg = encode_preview_jpeg(&thumb, 50).unwrap();
        assert!(jpeg.len() > 2);
        // JPEG SOI marker.
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    }
}
