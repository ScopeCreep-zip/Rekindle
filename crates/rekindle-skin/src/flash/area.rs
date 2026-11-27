//! The small-area clause of WCAG 2.2's "general flash and red flash
//! thresholds" (`.claude/plans/skins/raw/c/specs/wcag22.txt:1515-1530`):
//!
//! > the combined area of flashes occurring concurrently occupies no more
//! > than a total of .006 steradians within any 10 degree visual field on
//! > the screen (25% of any 10 degree visual field on the screen) at typical
//! > viewing distance
//!
//! > Note 1: For general software or web content, using a 341 x 256 pixel
//! > rectangle anywhere on the displayed screen area when the content is
//! > viewed at 1024 x 768 pixels will provide a good estimate of a 10 degree
//! > visual field […] This resolution of 75 - 85 ppi is known to be lower,
//! > and thus more conservative than the nominal CSS pixel resolution of 96
//! > ppi
//!
//! The field is a 341 × 256 rectangle in CSS px (at 96 ppi a CSS px is
//! physically smaller than a 75–85 ppi px, so the same count is a smaller
//! angle: conservative, as Note 1 says) and the limit is 25 % of it,
//! 21,824 px. "Combined area […] concurrently" is the Understanding
//! document's "total area that is actually flashing at the same time": the
//! pixels over the threshold inside one one-second window, counted
//! together whether contiguous or not.
//!
//! The area is measured as the image is rendered, which the validator knows
//! only as an upper bound. Region images are drawn nearest neighbour
//! (`image-rendering: pixelated`), so each axis is a sequence of source
//! indices with a rendered extent each: `tile` repeats the image at 1:1,
//! `stretch` scales it to a box that grows with the window (unbounded, so
//! one flashing source pixel can fill the field), `nine-slice` keeps its
//! insets at 1:1 (or smaller, when the box is smaller than the insets) and
//! stretches the middle. A flashing pixel on an unbounded row or column
//! breaks the threshold; otherwise a rendered pixel is at most one CSS px,
//! and the field's count is the flashing pixels it can cover.

use super::{AxisFit, DrawnAs};

/// Note 1's 10° field, CSS px.
const FIELD: (usize, usize) = (341, 256);

/// 25 % of the field: .006 sr.
pub(super) const MAX_AREA: u64 = (FIELD.0 * FIELD.1 / 4) as u64;

/// One axis as rendered: the source index at each rendered position,
/// whether that position's extent is unbounded, and how many positions a
/// field may start at.
struct Axis {
    idx: Vec<usize>,
    unbounded: Vec<bool>,
    starts: usize,
}

/// The rendered sequence of an axis of `n` source pixels under `fit`.
fn axis(fit: AxisFit, n: usize, field: usize) -> Axis {
    match fit {
        AxisFit::Tile => {
            // Every phase of the repeat, and a field's span past it.
            let len = n + field + 1;
            Axis {
                idx: (0..len).map(|i| i % n).collect(),
                unbounded: vec![false; len],
                starts: n,
            }
        }
        AxisFit::Stretch => Axis {
            idx: (0..n).collect(),
            unbounded: vec![true; n],
            starts: n,
        },
        AxisFit::Slice { start, end } => {
            let (a, b) = (start as usize, end as usize);
            let middle = |i: usize| a + b < n && i >= a && i < n - b;
            Axis {
                idx: (0..n).collect(),
                unbounded: (0..n).map(middle).collect(),
                starts: n,
            }
        }
    }
}

/// The rendered positions a field starting at each position covers, as
/// `[s, e)`: the first position whole (the field may begin at its far
/// edge), then each next one while the positions between are narrower
/// than the field. An unbounded position always ends the window.
fn windows(ax: &Axis, field: usize) -> Vec<(usize, usize)> {
    (0..ax.starts)
        .map(|s| {
            let mut e = s + 1;
            let mut between = 0usize;
            while e < ax.idx.len() && between < field {
                if ax.unbounded[e] {
                    e += 1;
                    break;
                }
                between += 1;
                e += 1;
            }
            (s, e)
        })
        .collect()
}

/// Whether the pixels `flagged` (row-major, `width` × `height`) cover more
/// than [`MAX_AREA`] of some field when the image is drawn by `drawn_as`.
pub(super) fn exceeds(width: usize, height: usize, flagged: &[bool], drawn_as: DrawnAs) -> bool {
    let ax = axis(drawn_as.x, width, FIELD.0);
    let ay = axis(drawn_as.y, height, FIELD.1);
    // Unbounded source rows and columns: the sequences above list every
    // source index once on those axes.
    let wide_col = |x: usize| ax.unbounded.get(x).copied().unwrap_or(false);
    let wide_row = |y: usize| ay.unbounded.get(y).copied().unwrap_or(false);
    for y in 0..height {
        for x in 0..width {
            if flagged[y * width + x] && (wide_col(x) || wide_row(y)) {
                return true;
            }
        }
    }
    let xw = windows(&ax, FIELD.0);
    let yw = windows(&ay, FIELD.1);
    // Per source row, the flagged pixels each column window covers.
    let mut rows = vec![0u32; height * xw.len()];
    let mut prefix = vec![0u32; ax.idx.len() + 1];
    for y in 0..height {
        for (i, x) in ax.idx.iter().enumerate() {
            prefix[i + 1] = prefix[i] + u32::from(flagged[y * width + x]);
        }
        for (k, (s, e)) in xw.iter().enumerate() {
            rows[y * xw.len() + k] = prefix[*e] - prefix[*s];
        }
    }
    let mut column = vec![0u64; ay.idx.len() + 1];
    for k in 0..xw.len() {
        for (i, y) in ay.idx.iter().enumerate() {
            column[i + 1] = column[i] + u64::from(rows[y * xw.len() + k]);
        }
        if yw.iter().any(|(s, e)| column[*e] - column[*s] > MAX_AREA) {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "area_tests.rs"]
mod tests;
