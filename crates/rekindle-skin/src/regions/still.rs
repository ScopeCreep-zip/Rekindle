//! The static first frame of an animated region image (plan E7.7.2
//! "Apply": `@layer a11y` "swaps animated regions for the Rust-emitted
//! static first frame under `prefers-reduced-motion`").
//!
//! A region GIF is a canonical one (`crate::images`: a looping GIF of its
//! composited frames). One with more than one frame gets a still: its
//! first frame composited onto the canvas, as an RGBA8 PNG. The still is
//! derived from the canonical bytes alone, so [`crate::regions::css::skin_css`]
//! and [`crate::serve::body`] compute the same bytes and the same BLAKE3
//! address, and it is served under that address like every other image.
//! A one-frame GIF does not move and gets none.

use std::collections::BTreeMap;

use rekindle_image::{
    decode_limited, encode_png, DecodeLimits, DynamicImage, Frames, ImageError, ImageKind, TooLarge,
};
use rekindle_types::dto::skin::SkinManifest;

use super::slots::slot_of;
use crate::images::extension;
use crate::limits::{MAX_IMAGE_ALLOC, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};

fn limits(frames: Frames) -> DecodeLimits {
    DecodeLimits {
        max_width: MAX_IMAGE_SIDE,
        max_height: MAX_IMAGE_SIDE,
        max_pixels: MAX_IMAGE_PIXELS,
        max_alloc: MAX_IMAGE_ALLOC,
        frames,
    }
}

/// The still of the GIF `gif`: its first frame as a PNG, or `None` for a
/// GIF of one frame.
///
/// Whether it moves is read with a one-frame limit, which `rekindle-image`
/// refuses at the second frame (`TooLarge::Frames`), so no frame past the
/// second is ever decoded here.
///
/// # Errors
/// The GIF does not decode, or the PNG does not encode (never for a
/// canonical GIF).
pub fn first_frame(gif: &[u8]) -> Result<Option<Vec<u8>>, ImageError> {
    match decode_limited(gif, ImageKind::Gif, &limits(Frames::AtMost(1))) {
        Ok(_) => return Ok(None),
        Err(ImageError::TooLarge(TooLarge::Frames { .. })) => {}
        Err(other) => return Err(other),
    }
    let first = decode_limited(gif, ImageKind::Gif, &limits(Frames::First))?.into_first();
    encode_png(&DynamicImage::ImageRgba8(first.into_rgba8())).map(Some)
}

/// Every painted region's animated image (normal and state) in `images`
/// (the canonical files by name), mapped to its still.
///
/// # Errors
/// What [`first_frame`] returns. A region image `images` lacks is left to
/// the caller, which addresses the image itself.
pub fn stills(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<String, Vec<u8>>, ImageError> {
    let mut out = BTreeMap::new();
    let painted = manifest
        .regions
        .iter()
        .filter(|(id, _)| slot_of(id).is_some_and(|s| s.css));
    for (_, region) in painted {
        for path in std::iter::once(&region.image).chain(region.states.values()) {
            if out.contains_key(path) || extension(path).as_deref() != Some("gif") {
                continue;
            }
            let Some(bytes) = images.get(path) else {
                continue;
            };
            if let Some(still) = first_frame(bytes)? {
                out.insert(path.clone(), still);
            }
        }
    }
    Ok(out)
}
