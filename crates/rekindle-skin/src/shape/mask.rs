//! A layer's mask: the 1-bit plane of the pixels opaque on their own
//! (alpha at least [`ALPHA_OPAQUE`]: a GIF's transparent index decodes to
//! alpha 0, every other index to 255; a PNG's partial alpha is cut at half,
//! research §7.3), and the faint pixels (alpha 1 to `ALPHA_OPAQUE - 1`)
//! with their alpha. The window decides a pixel from the *composited* alpha
//! of every layer laid on it, as the webview paints them (source-over,
//! `A = 1 − Π(1 − aᵢ)`, [`super::silhouette`]): a layer opaque on its own
//! decides alone, and two faint layers can together reach the threshold,
//! so the faint pixels travel with the mask.
//! A mask is never stored: [`of_layout`] derives every tile's from the
//! package's image at check and serve time (one source for the shape).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinError, SkinRule, WindowLayout};

use crate::images::decode_png;

/// The alpha from which a pixel is part of the window.
pub const ALPHA_OPAQUE: u8 = 128;

/// A faint pixel: (x, y, alpha), alpha in `1..ALPHA_OPAQUE`.
pub type Faint = (u32, u32, u8);

/// One bit per pixel, rows of `ceil(width / 8)` bytes, most significant
/// bit first; 1 is opaque. The faint pixels, sorted by row then column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    width: u32,
    height: u32,
    bits: Vec<u8>,
    faint: Vec<Faint>,
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
        let mut faint = Vec::new();
        for (x, y, px) in image.enumerate_pixels() {
            match px.0[3] {
                0 => {}
                a if a >= ALPHA_OPAQUE => {
                    bits[y as usize * s + (x / 8) as usize] |= 0x80 >> (x % 8);
                }
                a => faint.push((x, y, a)),
            }
        }
        Self {
            width,
            height,
            bits,
            faint,
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

    /// Row `y`'s faint pixels, left to right.
    #[must_use]
    pub fn faint_row(&self, y: u32) -> &[Faint] {
        let from = self.faint.partition_point(|f| f.1 < y);
        let to = self.faint.partition_point(|f| f.1 <= y);
        &self.faint[from..to]
    }

    /// A mask from its packed rows ([`Self::bits`]) and faint pixels
    /// ([`Self::faint`]); `None` when `bits` is not
    /// `ceil(width / 8) * height` bytes, or a faint pixel lies outside the
    /// mask, on an opaque bit, out of row-then-column order or with an
    /// alpha outside `1..ALPHA_OPAQUE`.
    #[must_use]
    pub fn from_parts(width: u32, height: u32, bits: Vec<u8>, faint: Vec<Faint>) -> Option<Self> {
        let mask = (bits.len() == stride(width).checked_mul(height as usize)?).then_some(Self {
            width,
            height,
            bits,
            faint: Vec::new(),
        })?;
        let ordered = faint
            .windows(2)
            .all(|p| (p[0].1, p[0].0) < (p[1].1, p[1].0));
        let valid = faint.iter().all(|&(x, y, a)| {
            x < width && y < height && a > 0 && a < ALPHA_OPAQUE && !mask.get(x, y)
        });
        (ordered && valid).then_some(Self { faint, ..mask })
    }

    /// The faint pixels, sorted by row then column.
    #[must_use]
    pub fn faint(&self) -> &[Faint] {
        &self.faint
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
/// canonical images), then the mask of each normal piece (in slot order)
/// of each node of the layout's tree that joins the outline ([`super::outline_nodes`]), in
/// that order; `field` names the layout in errors.
///
/// # Errors
/// A tile or node whose image is missing or does not decode, or a tile
/// image that is not the tile's declared size.
pub fn of_layout(
    field: &str,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Mask>, SkinError> {
    let decode = |image: &str, field: String| -> Result<RgbaImage, SkinError> {
        let bytes = images
            .get(image)
            .ok_or_else(|| SkinError::new(image, SkinRule::MissingEntry { field }))?;
        decode_png(bytes).map_err(|rule| SkinError::new(image, rule))
    };
    let mut out = Vec::new();
    for (i, t) in layout.tiles.iter().enumerate() {
        let field = format!("{field}.tiles.{i}");
        let img = decode(&t.image, field.clone())?;
        if img.dimensions() != (u32::from(t.width), u32::from(t.height)) {
            return Err(SkinError::new(
                field.as_str(),
                SkinRule::BadValue {
                    field: field.clone(),
                    expected: "the image's width and height".to_owned(),
                },
            ));
        }
        out.push(Mask::of(&img));
    }
    if let Some(tree) = &layout.tree {
        for (n, _) in super::outline_nodes(tree, 0, 0) {
            for (slot, piece) in n.art.iter().flat_map(|a| a.normal.iter()) {
                let img = decode(
                    &piece.image,
                    format!("{field}.tree.{}.{}", n.id, slot.key()),
                )?;
                out.push(Mask::of(&img));
            }
        }
    }
    Ok(out)
}
