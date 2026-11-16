//! What the importers' own bounded decoders share (plan E7.7.3: Winamp's
//! RLE BMP, [`super::winamp::bmp`], and Clist_modern's TGA,
//! [`super::miranda::tga`]): the error, little-endian reads that are
//! bounds-checked, and the validator's image limits checked before
//! anything is allocated.

use super::ImportError;
use crate::limits::{MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};

/// A refused or malformed `format` image `file`.
pub(crate) fn refused(format: &str, file: &str, detail: impl std::fmt::Display) -> ImportError {
    ImportError::Image {
        file: file.to_owned(),
        detail: format!("{format}: {detail}"),
    }
}

/// The little-endian `u16` at `at`, `None` past the end.
pub(crate) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// `Err` naming why a `width`×`height` image is outside the validator's
/// image limits (each side `1..=MAX_IMAGE_SIDE`, at most
/// `MAX_IMAGE_PIXELS`).
pub(crate) fn within_limits(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(format!("{width}x{height} is outside 1..={MAX_IMAGE_SIDE}"));
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("too many pixels".to_owned());
    }
    Ok(())
}
