//! The title's label backing over the titlebar strip (plan E7.7.7, the
//! owner's "every skinned title text sits on an opaque dark box even
//! where the strip already gives it contrast, which hides the strip art").
//!
//! The titlebar is a labelled slot ([`super::slots`]): where its art fails
//! the title, the strip paints the slot's `textBacking` once, as a band
//! behind its text row ([`super::css`]), so the text never depends on a
//! bitmap it was not measured against (WCAG 2.2 F83). That backing exists for one pair of the contrast model: the
//! title colour (`titlebar.text`) over the strip, held to SC 1.4.3 (the
//! title is 11 px semibold, `00-primitives.css` `.xfire-titlebar-text`,
//! so 4.5:1). Here the strip's own pixels are measured for that pair: the
//! pixels the title can cover, after the strip's 3-slice is scaled to the
//! chrome height ([`super::css`]'s strip), every window width from its
//! minimum to its maximum, every image (normal and inactive), every frame,
//! over a black and a white desktop. When the worst of them meets the
//! floor the strip has no band for that skin; otherwise the band is set
//! (a shaped window frame always sets it, `crate::shape`). The titlebar
//! buttons' glyphs share the band's row: this measures only the title.
//!
//! The title's box (`00-rules.css` `.xfire-titlebar`, `00-primitives.css`
//! `.xfire-titlebar-text`): it starts at the bar's 8 px padding and is at
//! most half the bar's content width; its line box is 11 px × the
//! preflight line height 1.5, centred in the chrome height. Every column
//! a title of any length can reach is measured, so a long title is
//! covered.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionFit, SkinManifest, TokenValue};

use super::css::{backing_property, band_property};
use super::{frames, slot_of, worst_contrast, Tokens, WINDOWS};
use crate::access::scaled;
use crate::registry::TokenEntry;

/// The titlebar strip slot.
pub const TITLE_REGION: &str = "titlebar";
/// `.xfire-titlebar`'s horizontal padding, CSS px.
const PAD_X: u32 = 8;
/// Half the title's line box (11 px × line height 1.5 = 16.5 px), in
/// quarter pixels (a row centre `y + 0.5` is `4y + 2` of them).
const LINE_HALF_PX: u32 = 33;

/// The title colour against the strip's worst pixel under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleGround {
    /// The worst contrast measured, × 100.
    pub worst_x100: u32,
    /// The pair's floor (SC 1.4.3), × 100.
    pub required_x100: u32,
}

impl TitleGround {
    /// Whether the strip alone carries the title: its backing is dropped.
    #[must_use]
    pub fn bare(&self) -> bool {
        self.worst_x100 >= self.required_x100
    }
}

#[cfg(test)]
thread_local! {
    /// The [`strip_col`] evaluations on this thread: the unit of the
    /// title's column work, which tests hold to the strip's width per
    /// window width (`mark_title_cols` steps per source column, never per
    /// output column).
    pub(crate) static COL_EVALS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The source column under output column `x` of a strip `iw` px wide
/// and `ih` tall with caps `left` and `right`, drawn `out` × `chrome` px
/// with its caps scaled by `chrome / ih` (CSS Backgrounds 3 §6.3 shrinks
/// both caps in proportion when they overlap). Lengths are in units of
/// `1 / (2 ih)` px so pixel centres stay exact integers.
fn strip_col(
    x: u32,
    out: u32,
    (iw, ih): (u32, u32),
    (left, right): (u32, u32),
    chrome: u32,
) -> u32 {
    #[cfg(test)]
    COL_EVALS.with(|n| n.set(n.get() + 1));
    let (iw64, ih64, ch) = (u64::from(iw), u64::from(ih), u64::from(chrome));
    let w = 2 * u64::from(out) * ih64;
    let (mut lw, mut rw) = (2 * u64::from(left) * ch, 2 * u64::from(right) * ch);
    if lw + rw > w {
        lw = lw * w / (lw + rw);
        rw = w - lw;
    }
    let c = (2 * u64::from(x) + 1) * ih64;
    let src = if c < lw {
        c * u64::from(left) / lw
    } else if c >= w - rw {
        iw64 - u64::from(right) + (c - (w - rw)) * u64::from(right) / rw.max(1)
    } else {
        u64::from(left) + (c - lw) * (iw64 - u64::from(left) - u64::from(right)) / (w - lw - rw)
    };
    u32::try_from(src.min(iw64 - 1)).unwrap_or(iw - 1)
}

/// The smallest output column whose centre `(2x + 1) ih` reaches `t`
/// (units of [`strip_col`]).
fn first_col(t: u64, ih: u64) -> u64 {
    t.saturating_sub(ih).div_ceil(2 * ih)
}

/// Marks in `hit` every source column [`strip_col`] gives for the output
/// columns `xs` at width `out` — the same set as evaluating each column,
/// in time bounded by the strip's width rather than the window's: within
/// each piece (left cap, middle, right cap) the map is one linear floor,
/// so a piece drawn at least as wide as its source hits every source
/// column between its ends (a step of at most one), and only a piece
/// drawn narrower than its source (fewer output columns than source
/// columns) is walked column by column (plan E7.7.7; the per-column walk
/// over every window width took seconds per stylesheet, past the Host's
/// answer deadline, so the skin was never applied).
fn mark_title_cols(
    hit: &mut [bool],
    xs: std::ops::Range<u32>,
    out: u32,
    (iw, ih): (u32, u32),
    (left, right): (u32, u32),
    chrome: u32,
) {
    let (ih64, ch) = (u64::from(ih), u64::from(chrome));
    let w = 2 * u64::from(out) * ih64;
    let (mut lw, mut rw) = (2 * u64::from(left) * ch, 2 * u64::from(right) * ch);
    if lw + rw > w {
        lw = lw * w / (lw + rw);
        rw = w - lw;
    }
    let (a, b) = (first_col(lw, ih64), first_col(w - rw, ih64));
    let middle = u64::from(iw - left - right);
    // (output columns, source span × 2 ih / output span) per piece: a step
    // of at most one source column when `src × 2 ih <= drawn`.
    let pieces = [
        (0..a, u64::from(left), lw),
        (a..b, middle, w - lw - rw),
        (b..u64::MAX, u64::from(right), rw),
    ];
    let (x0, x1) = (u64::from(xs.start), u64::from(xs.end));
    for (range, src, drawn) in pieces {
        let (lo, hi) = (range.start.max(x0), range.end.min(x1));
        if lo >= hi {
            continue;
        }
        let col = |x: u64| {
            strip_col(
                u32::try_from(x).unwrap_or(u32::MAX),
                out,
                (iw, ih),
                (left, right),
                chrome,
            )
        };
        if src * 2 * ih64 <= drawn {
            for c in col(lo)..=col(hi - 1) {
                hit[c as usize] = true;
            }
        } else {
            for x in lo..hi {
                hit[col(x) as usize] = true;
            }
        }
    }
}

/// The worst contrast of `text` over the pixels of `image` (a horizontal
/// 3-slice strip) a title can cover at `chrome` px tall, every window
/// width; `None` when `fit` is not such a strip.
#[must_use]
pub fn strip_worst(
    image: &RgbaImage,
    fit: RegionFit,
    chrome: u32,
    tint: Option<[u8; 4]>,
    text: [u8; 4],
) -> Option<f64> {
    let RegionFit::NineSlice(i) = fit else {
        return None;
    };
    let (iw, ih) = image.dimensions();
    let (left, right) = (u32::from(i.left), u32::from(i.right));
    if i.top != 0 || i.bottom != 0 || ih == 0 || chrome == 0 || left + right >= iw {
        return None;
    }
    let mut hit = vec![false; iw as usize];
    for win in &WINDOWS {
        for out in win.min.0..=win.max.0.max(win.min.0) {
            let span = out.saturating_sub(2 * PAD_X) / 2;
            mark_title_cols(
                &mut hit,
                PAD_X..PAD_X + span,
                out,
                (iw, ih),
                (left, right),
                chrome,
            );
        }
    }
    let cols: Vec<u32> = (0..iw).filter(|&c| hit[c as usize]).collect();
    // The rows whose centre is inside the line box centred in the chrome.
    let (lo, hi) = (
        (2 * chrome).saturating_sub(LINE_HALF_PX),
        2 * chrome + LINE_HALF_PX,
    );
    let rows: BTreeSet<u32> = (0..chrome)
        .filter(|y| (4 * y + 2) > lo && (4 * y + 2) < hi)
        .filter_map(|y| {
            u32::try_from((u64::from(y) * 2 + 1) * u64::from(ih) / (u64::from(chrome) * 2)).ok()
        })
        .map(|r| r.min(ih - 1))
        .collect();
    let rows: Vec<u32> = rows.into_iter().collect();
    Some(worst_contrast(image, &cols, &rows, tint, [0; 4], text))
}

/// The title's ground on the resolved variant `resolved`: `None` when the
/// skin paints no titlebar strip (or not as a 3-slice strip), so the
/// backing stays.
#[must_use]
pub fn title_ground(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> Option<TitleGround> {
    let region = manifest.regions.get(TITLE_REGION)?;
    let slot = slot_of(TITLE_REGION)?;
    let tokens = Tokens::new(resolved);
    let chrome = tokens.dims.get("size.chrome-height").copied()?;
    let tint = match &region.tint {
        Some(t) => Some(tokens.colours.get(t.as_str()).copied()?),
        None => None,
    };
    let mut worst = f64::INFINITY;
    let mut required = 0u32;
    for t in slot.text {
        let text = tokens.colours.get(t.token).copied()?;
        required = required.max(if t.large { 300 } else { 450 });
        for path in std::iter::once(&region.image).chain(region.states.values()) {
            for frame in frames(path, images.get(path)?).ok()? {
                worst = worst.min(strip_worst(&frame, region.fit, chrome, tint, text)?);
            }
        }
    }
    Some(TitleGround {
        worst_x100: scaled(worst, 100.0),
        required_x100: required,
    })
}

/// The skin layer setting the titlebar's band (the title's backing, once
/// behind its text row: [`super::css`]) unless the strip carries the title
/// on `resolved`; nothing for a skin without a titlebar strip.
#[must_use]
pub fn title_css(
    manifest: &SkinManifest,
    images: &BTreeMap<String, Vec<u8>>,
    resolved: &[(&TokenEntry, TokenValue)],
) -> String {
    if !manifest.regions.contains_key(TITLE_REGION) {
        return String::new();
    }
    match title_ground(manifest, images, resolved) {
        Some(g) if g.bare() => String::new(),
        _ => format!(
            "@layer skin {{\n  :root {{\n    {}: var({});\n  }}\n}}\n",
            band_property(TITLE_REGION),
            backing_property(TITLE_REGION)
        ),
    }
}

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
