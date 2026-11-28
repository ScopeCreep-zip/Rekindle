//! Where each labelled slot's text is drawn over its art (plan E7.7.7 V2
//! rest: measure only the pixels under the labels, as [`super::title`]
//! does for the title).
//!
//! A labelled slot's art is a `border-image` ([`super::css`]): a
//! nine-slice's edges at their pixel widths, all of them scaled by one
//! factor when they do not fit the box (CSS Backgrounds 3 §6.3,
//! `border-image-width`), the middle stretched; a `stretch` image over the
//! whole box; a `tile` image repeated with its tiles centred. A label's
//! text is laid out in a line box (`font-size` × the preflight line height
//! 1.5) inside its container's padding, so the pixels a label can cover
//! are the rows of that line box at every box height the slot takes and
//! the columns between the container's padding at every window width.
//! [`LabelBox`] holds that geometry per slot, from the app's CSS.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::RegionFit;

use super::slots::{Extent, WINDOWS};
use super::{axes, Axis};

/// Where a label's text line sits in its container's content box, in
/// quarter CSS px.
#[derive(Debug, Clone, Copy)]
pub enum Line {
    /// Centred (flex `align-items: center`, or a button's content).
    Centred(u32),
    /// At the content box's top (block flow, `align-items: baseline`).
    Top(u32),
    /// Anywhere in the content box (several lines, or labels that fill it).
    Fill,
}

/// A labelled slot's label geometry: its container's padding (CSS px)
/// and where the text line sits in what is left.
#[derive(Debug, Clone, Copy)]
pub struct LabelBox {
    /// Horizontal padding before the first label's text, each side.
    pub pad_x: u32,
    pub pad_top: u32,
    pub pad_bottom: u32,
    pub line: Line,
}

/// 11 px text (`font-size: 11px` × 1.5), in quarter px.
const LINE_11: u32 = 66;
/// 10 px text.
const LINE_10: u32 = 60;
/// A 14 px glyph (`.header-add-btn`, `02-form-primitives.css:36`).
const LINE_14: u32 = 84;

const fn label(pad_x: u32, pad_top: u32, pad_bottom: u32, line: Line) -> LabelBox {
    LabelBox {
        pad_x,
        pad_top,
        pad_bottom,
        line,
    }
}

/// The label geometry of labelled slot `id` (`regions.toml` `labels`).
/// A slot whose labels are not traced here is measured over its whole
/// box (`Fill`, no padding): every pixel some label may cover.
#[must_use]
pub fn label_box(id: &str) -> LabelBox {
    match id {
        // `.xfire-titlebar` (`00-rules.css:132`): 0 8 px padding, its 11 px
        // title centred ([`super::title`] measures the title's columns).
        // `.status-bar` (`01-form-primitives.css:82`): the same, 0 8 px
        // padding, centred, 11 px.
        "titlebar" | "statusbar" => label(8, 0, 0, Line::Centred(LINE_11)),
        // `.menu-bar-item` (`08-community-events.css:179`): a button the
        // bar's height, 0 10 px padding, its 11 px text centred.
        "buddylist.menubar" => label(10, 0, 0, Line::Centred(LINE_11)),
        // `.identity-bar` (`04-member-profile-popup.css:281`): a column of
        // lines in 6 10 2 px padding.
        "buddylist.infobar" => label(10, 6, 2, Line::Fill),
        // `.buddy-group-header` (`00-rules.css:369`), `.pending-header`
        // (`04-member-profile-popup.css:355`): 4 12 px padding, 11 px,
        // `align-items: baseline`.
        "buddylist.group-header" => label(12, 4, 4, Line::Top(LINE_11)),
        // `.action-bar` (`05-member-profile-popup.css:127`): 4 8 px
        // padding, its buttons fill the rest.
        "buddylist.actionbar" => label(8, 4, 4, Line::Fill),
        // `.chat-peer-status` (`06-member-profile-popup.css:189`): 0 12 px
        // padding, centred; its tallest labels are the 24 px call and
        // safety buttons (`:216`, `:341`).
        "chat.toolbar" => label(12, 0, 0, Line::Centred(96)),
        // `.group-chat-members` (`17-group-dm.css:73`): no padding, one
        // line of 11 px names (`:85`) at its top.
        "chat.group-header" => label(0, 0, 0, Line::Top(LINE_11)),
        // `.community-sidebar-header` (`02-form-primitives.css:58`): 8 12 px
        // padding, centred; its tallest label is the 14 px add button.
        "community.sidebar-header" => label(12, 8, 8, Line::Centred(LINE_14)),
        // `.category-header` (`09-emoji-picker.css:96`): 8 8 4 px padding,
        // centred, 10 px.
        "community.category-header" => label(8, 8, 4, Line::Centred(LINE_10)),
        // `.community-channel-header` (`02-form-primitives.css:407`): 0 12
        // px padding, centred; its tallest label is the 14 px add button.
        "community.channel-header" => label(12, 0, 0, Line::Centred(LINE_14)),
        // `.member-group-header` (`00-primitives.css:182`): 8 12 4 px
        // padding, 10 px, block flow.
        "community.member-group-header" => label(12, 8, 4, Line::Top(LINE_10)),
        _ => label(0, 0, 0, Line::Fill),
    }
}

/// The CSS scale of a nine-slice's widths in a `w` × `h` box (CSS
/// Backgrounds 3 §6.3: one factor for all four when a pair overflows).
fn width_scale((ax, ay): (Axis, Axis), w: u32, h: u32) -> f64 {
    let fit = |axis: Axis, out: u32| match axis {
        Axis::Slice(a, b) if a + b > out => f64::from(out) / f64::from(a + b),
        _ => 1.0,
    };
    fit(ax, w).min(fit(ay, h))
}

/// Mark the source indices an axis of `src` px draws into the output span
/// `[lo, hi)` of `out` px, a nine-slice's widths scaled by `f`.
fn mark(set: &mut [bool], axis: Axis, (lo, hi): (u32, u32), out: u32, scale: f64) {
    let f = scale;
    let src = u32::try_from(set.len()).unwrap_or(u32::MAX);
    if src == 0 || lo >= hi {
        return;
    }
    let mut put = |i: u32| set[usize::try_from(i.min(src - 1)).unwrap_or(0)] = true;
    let (left, right) = match axis {
        Axis::Tile => {
            // Tiles centred: output `x` shows `(x - (out - src) / 2) mod src`.
            if hi - lo >= src {
                (0..src).for_each(&mut put);
            } else {
                let shift = i64::from(out) - i64::from(src);
                for x in lo..hi {
                    let i = (2 * i64::from(x) + 1 - shift).div_euclid(2);
                    put(u32::try_from(i.rem_euclid(i64::from(src))).unwrap_or(0));
                }
            }
            return;
        }
        Axis::Stretch => (0, 0),
        Axis::Slice(l, r) => (l, r),
    };
    let (sa, sb) = (f64::from(left) * f, f64::from(right) * f);
    let (mid_src, mid_out) = (src - left - right, f64::from(out) - sa - sb);
    let source = |x: u32| -> u32 {
        let c = f64::from(x) + 0.5;
        let i = if c < sa {
            c / f
        } else if c > f64::from(out) - sb {
            f64::from(src - right) + (c - (f64::from(out) - sb)) / f
        } else {
            f64::from(left) + (c - sa) * f64::from(mid_src) / mid_out
        };
        floor_index(i).min(src - 1)
    };
    // A run drawn at 1:1 or larger skips no source index, so its ends
    // bound it; a shrunk run is sampled pixel by pixel.
    let magnified = f >= 1.0 && mid_out >= f64::from(mid_src);
    if magnified {
        let (first, last) = (source(lo), source(hi - 1));
        (first..=last).for_each(&mut put);
    } else {
        (lo..hi).map(source).for_each(&mut put);
    }
}

/// `x.floor()` as an index (0 for a negative or non-finite `x`).
fn floor_index(x: f64) -> u32 {
    let mut i = 0u32;
    if x.is_finite() && x >= 1.0 {
        // Exact for the pixel ranges here (`MAX_IMAGE_SIDE`).
        let mut step = 1u32 << 15;
        while step > 0 {
            if f64::from(i + step) <= x {
                i += step;
            }
            step >>= 1;
        }
    }
    i
}

/// The output rows of a box `h` px tall a label's text line covers.
fn line_rows(g: LabelBox, h: u32) -> (u32, u32) {
    let (top, bottom) = (4 * g.pad_top, (4 * h).saturating_sub(4 * g.pad_bottom));
    let (lo, hi) = match g.line {
        Line::Fill => (top, bottom),
        Line::Top(q) => (top, (top + q).min(bottom)),
        Line::Centred(q) => {
            let mid = top.midpoint(bottom);
            (mid.saturating_sub(q / 2), (mid + q / 2).min(4 * h))
        }
    };
    // The rows whose centre (`4y + 2` quarter px) is inside the line.
    let first = (0..h).find(|y| 4 * y + 2 > lo).unwrap_or(h);
    let end = (0..h)
        .rev()
        .find(|y| 4 * y + 2 < hi)
        .map_or(first, |y| y + 1);
    (first, end.max(first))
}

/// Every box side an extent takes over every window (min to max).
fn sides(extent: Extent, chrome: u32, width: bool) -> Vec<(u32, u32)> {
    match extent {
        Extent::Window => WINDOWS
            .iter()
            .map(|w| {
                let (lo, hi) = if width {
                    (w.min.0, w.max.0)
                } else {
                    (w.min.1, w.max.1)
                };
                #[cfg(test)]
                crate::work::add(&crate::work::LABEL_SPANS, u64::from(hi.max(lo) - lo + 1));
                (lo, hi.max(lo))
            })
            .collect(),
        Extent::Token(_) => vec![(chrome, chrome)],
        Extent::Range { min, max } => vec![(min, max.max(min))],
    }
}

/// The source columns and rows an image of `iw` × `ih` under `fit` shows
/// under slot `id`'s labels, grouped by the width scale they are drawn at
/// (each group's columns × rows are pixels some label covers, a superset
/// when two boxes of one group differ); `None` for a fit whose insets
/// leave no middle.
#[must_use]
pub fn label_pixels(
    id: &str,
    (width, height): (Extent, Extent),
    chrome: u32,
    fit: RegionFit,
    (iw, ih): (u32, u32),
) -> Option<Vec<(Vec<u32>, Vec<u32>)>> {
    let ax = axes(fit, iw, ih)?;
    let g = label_box(id);
    let span = |axis: Axis, src: u32, pad: u32| match axis {
        // Past this length every further pixel repeats an index already drawn.
        Axis::Slice(a, b) => a + b + 2 * (src + pad + 1),
        Axis::Stretch => 2 * (pad + 1) * src + src,
        Axis::Tile => src + 2 * pad + 1,
    };
    let cap_w = span(ax.0, iw, g.pad_x);
    let widths: Vec<u32> = sides(width, chrome, true)
        .into_iter()
        .flat_map(|(lo, hi)| lo.max(1)..=hi.min(lo.max(cap_w)).max(1))
        .collect();
    let heights: Vec<u32> = sides(height, chrome, false)
        .into_iter()
        .flat_map(|(lo, hi)| lo.max(1)..=hi.max(1))
        .collect();
    // Each axis's own scale (1 unless its slices overflow the box); a
    // box's scale is the smaller of its two.
    let own = |w: u32, h: u32| width_scale(ax, w, h);
    let fw: Vec<f64> = widths.iter().map(|w| own(*w, u32::MAX)).collect();
    let fh: Vec<f64> = heights.iter().map(|h| own(u32::MAX, *h)).collect();
    let distinct = |v: &[f64]| -> Vec<f64> {
        let mut d: Vec<u64> = v.iter().map(|f| f.to_bits()).collect();
        d.sort_unstable();
        d.dedup();
        d.into_iter().map(f64::from_bits).collect()
    };
    let (dw, dh) = (distinct(&fw), distinct(&fh));
    let (n_w, n_h) = (
        usize::try_from(iw).unwrap_or(0),
        usize::try_from(ih).unwrap_or(0),
    );
    let mut groups: BTreeMap<u64, (Vec<bool>, Vec<bool>)> = BTreeMap::new();
    let fresh = || (vec![false; n_w], vec![false; n_h]);
    for (w, own_w) in widths.iter().zip(&fw) {
        for other in &dh {
            let f = own_w.min(*other);
            mark(
                &mut groups.entry(f.to_bits()).or_insert_with(fresh).0,
                ax.0,
                (g.pad_x.min(*w), w.saturating_sub(g.pad_x)),
                *w,
                f,
            );
        }
    }
    for (h, own_h) in heights.iter().zip(&fh) {
        for other in &dw {
            let f = own_h.min(*other);
            mark(
                &mut groups.entry(f.to_bits()).or_insert_with(fresh).1,
                ax.1,
                line_rows(g, *h),
                *h,
                f,
            );
        }
    }
    let idx = |v: &[bool]| -> Vec<u32> {
        (0u32..)
            .zip(v)
            .filter(|(_, on)| **on)
            .map(|(i, _)| i)
            .collect()
    };
    Some(
        groups
            .values()
            .map(|(c, r)| (idx(c), idx(r)))
            .filter(|(c, r)| !c.is_empty() && !r.is_empty())
            .collect(),
    )
}

/// The worst contrast of `text` over the pixels of `image` under slot
/// `id`'s labels; `None` for a fit whose insets leave no middle.
#[must_use]
pub fn label_worst(
    id: &str,
    extents: (Extent, Extent),
    chrome: u32,
    image: &RgbaImage,
    fit: RegionFit,
    tint: Option<[u8; 4]>,
    text: [u8; 4],
) -> Option<f64> {
    let groups = label_pixels(id, extents, chrome, fit, image.dimensions())?;
    Some(groups.iter().fold(f64::INFINITY, |worst, (cols, rows)| {
        worst.min(super::worst_contrast(image, cols, rows, tint, [0; 4], text))
    }))
}

#[cfg(test)]
#[path = "label_box_tests.rs"]
mod tests;
