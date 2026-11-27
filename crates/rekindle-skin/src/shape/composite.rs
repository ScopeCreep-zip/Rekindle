//! The composited alpha of a window pixel (plan SK0: the window's shape
//! follows what is painted). The webview paints every frame tile, then
//! every outline node's art ([`super::outline_nodes`]), source-over, so a
//! pixel's alpha is `A = 1 − Π(1 − aᵢ)` over every layer laid on it, and
//! the pixel is part of the window when `A` is at least
//! [`ALPHA_OPAQUE`]. A layer opaque on its own decides alone (its 1-bit
//! runs, [`super::Mask`]); only where two or more faint layers (alpha
//! below the threshold) overlap does a pixel need this per-pixel path, so
//! the cost is bounded by the faint pixels laid on a scanline.

use super::{Faint, ALPHA_OPAQUE};

/// One faint pixel laid on a scanline: (window x, layer in paint order,
/// alpha).
pub(super) type Laid = (i64, usize, u8);

/// Fixed-point one for the transparency product (integer arithmetic, so
/// the result is the same on every platform).
const ONE: u64 = 1 << 32;

/// Lays the faint pixels `src` of one source row of layer `layer`, a copy
/// at window x `at` that is `len` px wide, clipped to the window columns
/// `[lo, hi)`, onto `out`.
pub(super) fn lay(
    src: &[Faint],
    (at, len): (i64, i64),
    (lo, hi): (i64, i64),
    layer: usize,
    out: &mut Vec<Laid>,
) {
    for &(fx, _, a) in src {
        let (fx, x) = (i64::from(fx), at + i64::from(fx));
        if fx < len && x >= lo && x < hi {
            out.push((x, layer, a));
        }
    }
}

/// Whether the layers `alphas` composite to at least [`ALPHA_OPAQUE`]:
/// the transparency `Π(1 − aᵢ)`, each step rounded up (a pixel never
/// joins by rounding), at most `1 − ALPHA_OPAQUE / 255`.
fn composites_opaque(alphas: impl Iterator<Item = u8>) -> bool {
    let mut clear = ONE;
    for a in alphas {
        clear = (clear * u64::from(255 - a)).div_ceil(255);
    }
    clear * 255 <= u64::from(255 - ALPHA_OPAQUE) * ONE
}

/// Adds to `row` every pixel of `faint` (one scanline's laid faint pixels)
/// where two or more layers composite to at least [`ALPHA_OPAQUE`], as
/// one-pixel runs; the caller merges `row`.
pub(super) fn cover(faint: &mut [Laid], row: &mut Vec<(i64, i64)>) {
    if faint.len() < 2 {
        return;
    }
    faint.sort_unstable();
    for group in faint.chunk_by(|a, b| a.0 == b.0) {
        if group.len() > 1 && composites_opaque(group.iter().map(|l| l.2)) {
            row.push((group[0].0, group[0].0 + 1));
        }
    }
}
