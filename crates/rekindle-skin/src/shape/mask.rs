//! A tile's 1-bit mask: opaque where the pixel's alpha is at least
//! [`ALPHA_OPAQUE`] (a GIF's transparent index decodes to alpha 0, every
//! other index to 255; a PNG's partial alpha is cut at half, research §7.3).
//! A mask is never stored: [`of_layout`] derives every tile's from the
//! package's image at check and serve time (one source for the shape).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinError, SkinRule, WindowLayout};

use crate::images::decode_png;

/// The alpha from which a pixel is part of the window.
pub const ALPHA_OPAQUE: u8 = 128;

/// One bit per pixel, rows of `ceil(width / 8)` bytes, most significant
/// bit first; 1 is opaque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    width: u32,
    height: u32,
    bits: Vec<u8>,
}

fn stride(width: u32) -> usize {
    width.div_ceil(8) as usize
}

impl Mask {
    /// The mask of `image`'s alpha.
    #[must_use]
    pub fn of(image: &RgbaImage) -> Self {
        let (width, height) = image.dimensions();
        let s = stride(width);
        let mut bits = vec![0u8; s * height as usize];
        for (x, y, px) in image.enumerate_pixels() {
            if px.0[3] >= ALPHA_OPAQUE {
                bits[y as usize * s + (x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        Self {
            width,
            height,
            bits,
        }
    }

    /// Whether pixel (`x`, `y`) is opaque.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> bool {
        x < self.width
            && y < self.height
            && self.bits[y as usize * stride(self.width) + (x / 8) as usize] & (0x80 >> (x % 8))
                != 0
    }

    /// Row `y`'s opaque runs, `[start, end)`.
    #[must_use]
    pub fn runs(&self, y: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut start = None;
        for x in 0..=self.width {
            match (x < self.width && self.get(x, y), start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    out.push((s, x));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    /// A mask from its packed rows ([`Self::bits`]); `None` when `bits`
    /// is not `ceil(width / 8) * height` bytes.
    #[must_use]
    pub fn from_bits(width: u32, height: u32, bits: Vec<u8>) -> Option<Self> {
        (bits.len() == stride(width).checked_mul(height as usize)?).then_some(Self {
            width,
            height,
            bits,
        })
    }

    /// The packed rows: `ceil(width / 8)` bytes each, most significant
    /// bit first.
    #[must_use]
    pub fn bits(&self) -> &[u8] {
        &self.bits
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// Each tile's mask, decoded from its image in `images` (the package's
/// canonical images); `field` names the layout in errors.
///
/// # Errors
/// A tile whose image is missing, does not decode, or is not the tile's
/// declared size.
pub fn of_layout(
    field: &str,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Mask>, SkinError> {
    layout
        .tiles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let field = format!("{field}.tiles.{i}");
            let bytes = images.get(&t.image).ok_or_else(|| {
                SkinError::new(
                    t.image.as_str(),
                    SkinRule::MissingEntry {
                        field: field.clone(),
                    },
                )
            })?;
            let img = decode_png(bytes).map_err(|rule| SkinError::new(t.image.as_str(), rule))?;
            if img.dimensions() != (u32::from(t.width), u32::from(t.height)) {
                return Err(SkinError::new(
                    field.as_str(),
                    SkinRule::BadValue {
                        field: field.clone(),
                        expected: "the image's width and height".to_owned(),
                    },
                ));
            }
            Ok(Mask::of(&img))
        })
        .collect()
}
