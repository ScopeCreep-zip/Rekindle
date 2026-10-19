//! Re-encoding: pixels only, no metadata (text chunks, EXIF, ICC) carried
//! over from the source.

use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, Frame};

use crate::error::ImageError;

/// `image` scaled down (Lanczos3, aspect kept) so neither side exceeds
/// `max_side`; unchanged when it already fits.
#[must_use]
pub fn fit_within(image: DynamicImage, max_side: u32) -> DynamicImage {
    if image.width() > max_side || image.height() > max_side {
        image.resize(max_side, max_side, FilterType::Lanczos3)
    } else {
        image
    }
}

/// `image` as a PNG.
///
/// # Errors
/// [`ImageError::Encode`] when the encoder refuses it.
pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    image
        .write_with_encoder(PngEncoder::new(&mut out))
        .map_err(|e| ImageError::Encode(e.to_string()))?;
    Ok(out)
}

/// `image` as a lossless WebP (8-bit, alpha kept when it has one).
///
/// # Errors
/// [`ImageError::Encode`] when the encoder refuses it.
pub fn encode_webp(image: &DynamicImage) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    image
        .write_with_encoder(WebPEncoder::new_lossless(&mut out))
        .map_err(|e| ImageError::Encode(e.to_string()))?;
    Ok(out)
}

/// `frames` as a GIF that loops forever, each frame keeping its delay.
///
/// # Errors
/// [`ImageError::Encode`] when the encoder refuses a frame.
pub fn encode_gif(frames: Vec<Frame>) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut out);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| ImageError::Encode(e.to_string()))?;
        encoder
            .encode_frames(frames)
            .map_err(|e| ImageError::Encode(e.to_string()))?;
    }
    Ok(out)
}
