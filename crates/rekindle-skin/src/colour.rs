//! sRGB colour arithmetic for derivations and contrast: `color-mix(in
//! srgb, …)` compositing and WCAG 2.x relative luminance (sRGB transfer per
//! CSS Color 4).

/// `a` at `a_weight` thousandths, `b` at the rest, per channel (alpha too).
#[must_use]
pub fn mix(a: [u8; 4], b: [u8; 4], a_weight: u16) -> [u8; 4] {
    let w = u32::from(a_weight.min(1000));
    let mut out = [0u8; 4];
    for (o, (x, y)) in out.iter_mut().zip(a.iter().zip(b.iter())) {
        let v = (u32::from(*x) * w + u32::from(*y) * (1000 - w) + 500) / 1000;
        *o = u8::try_from(v).unwrap_or(u8::MAX);
    }
    out
}

/// `channel` scaled by `by` thousandths.
#[must_use]
pub fn scale(channel: u8, by: u16) -> u8 {
    let v = (u32::from(channel) * u32::from(by.min(1000)) + 500) / 1000;
    u8::try_from(v).unwrap_or(u8::MAX)
}

/// `top` composited over the opaque `under`.
#[must_use]
pub fn over(top: [u8; 4], under: [u8; 3]) -> [u8; 3] {
    let w = (u32::from(top[3]) * 1000 + 127) / 255;
    let m = mix(
        [top[0], top[1], top[2], 255],
        [under[0], under[1], under[2], 255],
        u16::try_from(w).unwrap_or(1000),
    );
    [m[0], m[1], m[2]]
}

/// The sRGB transfer function, one channel to linear light (WCAG 2.2).
pub(crate) fn linear(c: u8) -> f64 {
    linear_unit(unit(c))
}

/// One 8-bit channel as a gamma-encoded value in 0..=1.
fn unit(c: u8) -> f64 {
    f64::from(c) / 255.0
}

/// The sRGB transfer function on a gamma-encoded channel in 0..=1.
fn linear_unit(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
#[must_use]
pub fn luminance(rgb: [u8; 3]) -> f64 {
    luminance_unit(rgb.map(unit))
}

/// WCAG relative luminance of gamma-encoded channels in 0..=1 (the pair
/// model's composited colours, [`crate::access::model`]).
#[must_use]
pub fn luminance_unit(rgb: [f64; 3]) -> f64 {
    0.2126 * linear_unit(rgb[0]) + 0.7152 * linear_unit(rgb[1]) + 0.0722 * linear_unit(rgb[2])
}

/// The WCAG contrast ratio of two opaque colours.
#[must_use]
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    contrast_unit(a.map(unit), b.map(unit))
}

/// The WCAG contrast ratio of two opaque colours given as gamma-encoded
/// channels in 0..=1.
#[must_use]
pub fn contrast_unit(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (la, lb) = (luminance_unit(a), luminance_unit(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_vectors() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        // The accent fill's documented 4.54:1 under white text.
        let r = contrast([0x16, 0x7b, 0xc0], [255, 255, 255]);
        assert!((r - 4.54).abs() < 0.01, "{r}");
        assert_eq!(
            mix([255, 255, 255, 255], [0, 0, 0, 255], 500),
            [128, 128, 128, 255]
        );
        assert_eq!(over([255, 255, 255, 0], [10, 20, 30]), [10, 20, 30]);
    }
}
