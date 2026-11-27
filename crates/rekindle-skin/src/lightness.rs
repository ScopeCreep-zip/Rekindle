//! Oklab lightness (Ottosson, "A perceptual color space for image
//! processing", bottosson.github.io/posts/oklab, linear sRGB matrices) and
//! the minimal lightness move both walks share (plan E7.7.3): the import's
//! base walk (`import::walk`), the dialog walk
//! (`registry::dialog_walk`), the editor's suggestion and the role tags.

use rekindle_types::dto::skin::TokenValue;

use crate::access::scaled;

pub(crate) const STEPS: u32 = 20;

fn to_gamma(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    u8::try_from(scaled(v * 255.0 + 0.5, 1.0)).unwrap_or(u8::MAX)
}

type Matrix = [[f64; 3]; 3];

const TO_LMS: Matrix = [
    [0.412_221_470_8, 0.536_332_536_3, 0.051_445_992_9],
    [0.211_903_498_2, 0.680_699_545_1, 0.107_396_956_6],
    [0.088_302_461_9, 0.281_718_837_6, 0.629_978_700_5],
];
const TO_OKLAB: Matrix = [
    [0.210_454_255_3, 0.793_617_785, -0.004_072_046_8],
    [1.977_998_495_1, -2.428_592_205, 0.450_593_709_9],
    [0.025_904_037_1, 0.782_771_766_2, -0.808_675_766],
];
const FROM_OKLAB: Matrix = [
    [1.0, 0.396_337_777_4, 0.215_803_757_3],
    [1.0, -0.105_561_345_8, -0.063_854_172_8],
    [1.0, -0.089_484_177_5, -1.291_485_548],
];
const FROM_LMS: Matrix = [
    [4.076_741_662_1, -3.307_711_591_3, 0.230_969_929_2],
    [-1.268_438_004_6, 2.609_757_401_1, -0.341_319_396_5],
    [-0.004_196_086_3, -0.703_418_614_7, 1.707_614_701],
];

fn mat(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// Gamma-encoded sRGB to Oklab `[L, a, b]`.
#[must_use]
pub fn oklab(rgb: [u8; 3]) -> [f64; 3] {
    let lin = rgb.map(crate::colour::linear);
    let lms = mat(&TO_LMS, lin).map(f64::cbrt);
    mat(&TO_OKLAB, lms)
}

/// Oklab to gamma-encoded sRGB, clamped to the gamut per channel.
#[must_use]
pub fn from_oklab(lab: [f64; 3]) -> [u8; 3] {
    let lms = mat(&FROM_OKLAB, lab).map(|v| v.powi(3));
    mat(&FROM_LMS, lms).map(to_gamma)
}

/// The smallest lightness move of `original` (towards white when `up`)
/// that `passes`, or `None` when even the end does not.
///
/// # Errors
/// What `passes` returns.
pub fn walk<E>(
    original: [u8; 3],
    up: bool,
    passes: impl FnMut([u8; 3]) -> Result<bool, E>,
) -> Result<Option<[u8; 3]>, E> {
    walk_lab(oklab(original), up, passes)
}

/// [`walk`] from an Oklab start: its a and b stay, its lightness moves.
/// The dialog walk (`registry::dialog_walk`) starts here from a colour whose
/// chroma it has reduced.
///
/// # Errors
/// What `passes` returns.
pub fn walk_lab<E>(
    [l, a, b]: [f64; 3],
    up: bool,
    mut passes: impl FnMut([u8; 3]) -> Result<bool, E>,
) -> Result<Option<[u8; 3]>, E> {
    let end = if up { 1.0 } else { 0.0 };
    let at = |t: f64| from_oklab([l + (end - l) * t, a, b]);
    if !passes(at(1.0))? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..STEPS {
        let mid = f64::midpoint(lo, hi);
        if passes(at(mid))? {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Ok(Some(at(hi)))
}

/// An opaque colour token value.
#[must_use]
pub fn color([r, g, b]: [u8; 3]) -> TokenValue {
    TokenValue::Color { r, g, b, a: 255 }
}

#[cfg(test)]
#[path = "lightness_tests.rs"]
mod tests;
