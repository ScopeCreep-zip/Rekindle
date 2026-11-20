//! A tile's 1-bit mask: opaque where the pixel's alpha is at least
//! [`ALPHA_OPAQUE`] (a GIF's transparent index decodes to alpha 0, every
//! other index to 255; a PNG's partial alpha is cut at half, research §7.3).

use rekindle_image::RgbaImage;

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

    /// The hex text the package stores.
    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        self.bits.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// Parses `hex` as a `width × height` mask; `None` when its length or
    /// digits are wrong (lower-case only: one spelling per mask).
    #[must_use]
    pub fn from_hex(hex: &str, width: u32, height: u32) -> Option<Self> {
        let len = stride(width).checked_mul(height as usize)?;
        if hex.len() != len.checked_mul(2)? {
            return None;
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let bits = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
            .collect::<Option<Vec<u8>>>()?;
        Some(Self {
            width,
            height,
            bits,
        })
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

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }
}
