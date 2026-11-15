//! Xfire bitmaps: decoded under the validator's own image limits, the
//! `#FF00FF` colour key turned into alpha, a signed colour shift baked in,
//! and nine separate pieces composed into one `nine-slice` image.

use rekindle_image::{decode_limited, kind_of, DecodeLimits, Frames, RgbaImage};
use rekindle_types::dto::skin::Insets;

use super::super::ImportError;
use crate::limits::{MAX_IMAGE_ALLOC, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};

/// The legacy engine's transparent colour (pixel evidence, d1 §5).
pub const COLOUR_KEY: [u8; 3] = [0xff, 0x00, 0xff];

/// The nine piece types, row by row.
pub const NINE: [&str; 9] = [
    "TopLeft",
    "Top",
    "TopRight",
    "Left",
    "Middle",
    "Right",
    "BottomLeft",
    "Bottom",
    "BottomRight",
];

fn image_err(file: &str, detail: impl std::fmt::Display) -> ImportError {
    ImportError::Image {
        file: file.to_owned(),
        detail: detail.to_string(),
    }
}

/// Decodes `bytes` (GIF, PNG or JPEG, first frame) and keys `#FF00FF` out.
///
/// # Errors
/// [`ImportError::Image`] for an unknown kind or a refused decode.
pub fn decode(file: &str, bytes: &[u8]) -> Result<RgbaImage, ImportError> {
    let kind = kind_of(bytes).ok_or_else(|| image_err(file, "not an image"))?;
    let limits = DecodeLimits {
        max_width: MAX_IMAGE_SIDE,
        max_height: MAX_IMAGE_SIDE,
        max_pixels: MAX_IMAGE_PIXELS,
        max_alloc: MAX_IMAGE_ALLOC,
        frames: Frames::First,
    };
    let mut image = decode_limited(bytes, kind, &limits)
        .map_err(|e| image_err(file, e))?
        .into_first()
        .into_rgba8();
    colour_key(&mut image);
    Ok(image)
}

/// Every opaque `#FF00FF` pixel becomes fully transparent.
pub fn colour_key(image: &mut RgbaImage) {
    for p in image.pixels_mut() {
        if p.0[..3] == COLOUR_KEY {
            p.0 = [0, 0, 0, 0];
        }
    }
}

/// Adds `by` to every channel of every visible pixel, clamped; alpha
/// untouched, and a fully transparent (keyed) pixel stays `[0, 0, 0, 0]`.
pub fn shift(image: &mut RgbaImage, by: [i16; 3]) {
    if by == [0; 3] {
        return;
    }
    for p in image.pixels_mut().filter(|p| p.0[3] != 0) {
        for (c, d) in p.0.iter_mut().zip(by) {
            let v = (i16::from(*c) + d).clamp(0, 255);
            *c = u8::try_from(v).unwrap_or(u8::MAX);
        }
    }
}

fn copy_tiled(out: &mut RgbaImage, piece: &RgbaImage, x0: u32, y0: u32, w: u32, h: u32) {
    if piece.width() == 0 || piece.height() == 0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let p = *piece.get_pixel(x % piece.width(), y % piece.height());
            out.put_pixel(x0 + x, y0 + y, p);
        }
    }
}

/// The nine pieces (in [`NINE`] order) as one image and its insets: each
/// column as wide as its widest piece, each row as tall as its tallest,
/// each piece tiled into its cell from the top left (Xfire `Tile`).
///
/// # Errors
/// [`ImportError::Image`] when the result is too large to be a region.
pub fn compose(file: &str, pieces: &[RgbaImage; 9]) -> Result<(RgbaImage, Insets), ImportError> {
    let col = |c: usize| (0..3).map(|r| pieces[r * 3 + c].width()).max().unwrap_or(0);
    let row = |r: usize| {
        (0..3)
            .map(|c| pieces[r * 3 + c].height())
            .max()
            .unwrap_or(0)
    };
    let widths = [col(0), col(1), col(2)];
    let heights = [row(0), row(1), row(2)];
    let (w, h) = (widths.iter().sum::<u32>(), heights.iter().sum::<u32>());
    if w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return Err(image_err(file, "the composed frame is too large"));
    }
    let mut out = RgbaImage::new(w, h);
    let mut y0 = 0;
    for (r, &rh) in heights.iter().enumerate() {
        let mut x0 = 0;
        for (c, &cw) in widths.iter().enumerate() {
            copy_tiled(&mut out, &pieces[r * 3 + c], x0, y0, cw, rh);
            x0 += cw;
        }
        y0 += rh;
    }
    let inset = |v: u32| u16::try_from(v).map_err(|_| image_err(file, "inset too large"));
    let insets = Insets {
        top: inset(heights[0])?,
        right: inset(widths[2])?,
        bottom: inset(heights[2])?,
        left: inset(widths[0])?,
    };
    Ok((out, insets))
}

#[cfg(test)]
mod tests {
    use rekindle_image::Rgba;

    use super::*;

    #[test]
    fn key_and_shift() {
        let mut i = RgbaImage::from_pixel(2, 1, Rgba([0xff, 0, 0xff, 0xff]));
        i.put_pixel(1, 0, Rgba([100, 250, 10, 0xff]));
        colour_key(&mut i);
        assert_eq!(i.get_pixel(0, 0).0, [0, 0, 0, 0]);
        shift(&mut i, [30, 30, -30]);
        assert_eq!(i.get_pixel(1, 0).0, [130, 255, 0, 0xff]);
        assert_eq!(i.get_pixel(0, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn nine_pieces_compose() {
        let p = |w, h, v| RgbaImage::from_pixel(w, h, Rgba([v, v, v, 255]));
        let pieces = [
            p(6, 4, 1),
            p(4, 4, 2),
            p(6, 4, 3),
            p(6, 4, 4),
            p(24, 24, 5),
            p(6, 4, 6),
            p(6, 4, 7),
            p(4, 4, 8),
            p(6, 4, 9),
        ];
        let (img, insets) = compose("f", &pieces).unwrap();
        assert_eq!((img.width(), img.height()), (36, 32));
        assert_eq!(
            (insets.top, insets.right, insets.bottom, insets.left),
            (4, 6, 4, 6)
        );
        assert_eq!(img.get_pixel(20, 2).0[0], 2);
        assert_eq!(img.get_pixel(2, 20).0[0], 4);
        assert_eq!(img.get_pixel(35, 31).0[0], 9);
    }
}
