//! The titlebar strip (plan E7.7.7, the owner's "the corners of the top bar
//! skin seem torn"): a horizontal 3-slice whose box is the chrome height.
//!
//! Xfire draws its title strip (Classic v3 `MainTitleBar` 240 × 47,
//! xSLIMFIRE's 266 × 106) at its own height; our titlebar is
//! `--chrome-height` tall (`src/styles/theme/00-rules.css` `.xfire-titlebar`).
//! Drawn with the end caps at their pixel widths, the caps were squashed to
//! the chrome height but kept their full width, and xSLIMFIRE's 131 px caps
//! met in the middle of a narrow window, so the ends looked cut off. Here the
//! caps keep their aspect: each cap's width is its inset scaled by
//! `--chrome-height / image height` (CSS Backgrounds 3 §6.3, a `<length>`
//! border-image-width), so both caps sit whole inside the window and the
//! middle slice spans what is left.
//!
//! The strip is the titlebar's ground: its `-ground` property is
//! `transparent`, which the titlebar's own gradient reads
//! (`00-rules.css`), so the token gradient never shows through the caps'
//! transparent corner pixels (the blue triangles at both top corners).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::RegionFit;

use super::{dimensions, property};

/// The slots whose element is `--chrome-height` tall.
const CHROME_STRIPS: &[&str] = &["titlebar"];

/// Whether `id` is a chrome-height strip.
pub(super) fn is_strip(id: &str) -> bool {
    CHROME_STRIPS.contains(&id)
}

/// The custom property naming a strip's ground: `transparent` when the
/// skin paints the strip, unset otherwise (the app's gradient shows).
pub(super) fn ground_property(id: &str) -> String {
    format!("{}-ground", property(id, None))
}

/// The `border-image` value of slot `id`'s image `path` at `href`: a
/// chrome strip's 3-slice scaled to the chrome height, else
/// [`super::value`]. `None` when a strip image's header does not read.
pub(super) fn painted(
    id: &str,
    href: &str,
    path: &str,
    fit: RegionFit,
    images: &BTreeMap<String, Vec<u8>>,
) -> Option<String> {
    if !is_strip(id) {
        return Some(super::value(href, fit));
    }
    let (_, height) = dimensions(path, images)?;
    Some(value(href, fit, height).unwrap_or_else(|| super::value(href, fit)))
}

/// The `border-image` value of a strip image `height` px tall at `href`:
/// a horizontal 3-slice (`top` and `bottom` 0) gets its caps scaled to the
/// chrome height; any other fit is `None` (drawn as every region is).
pub(super) fn value(href: &str, fit: RegionFit, height: u32) -> Option<String> {
    let RegionFit::NineSlice(i) = fit else {
        return None;
    };
    if i.top != 0 || i.bottom != 0 || height == 0 {
        return None;
    }
    let cap = |inset: u16| format!("calc(var(--chrome-height) * {inset} / {height})");
    Some(format!(
        "url(\"{href}\") 0 {} 0 {} fill / 0 {} 0 {} / 0 stretch",
        i.right,
        i.left,
        cap(i.right),
        cap(i.left)
    ))
}

#[cfg(test)]
#[path = "strip_tests.rs"]
mod tests;
