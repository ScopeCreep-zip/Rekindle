//! The helper's refusals of a frame layout (plan SK0 S1, S4; research §6).
//!
//! A shaped window shows, and passes clicks to, whatever lies behind its
//! transparent pixels, so a hostile skin could frame another app's button
//! as Rekindle's (UI redressing; threat model). The rules:
//! 1. each tile's mask is its image's alpha (≥ [`super::ALPHA_OPAQUE`]):
//!    the shape is what is drawn, no more and no less;
//! 2. no frame tile reaches into the content rectangle, which is always
//!    opaque (security surfaces live there, `@layer sec`);
//! 3. no hole: every transparent pixel connects to the window's border, so
//!    only the outer frame band and overhangs are cut;
//! 4. at most [`MAX_RECTS`] rectangles at the largest window, so a skin
//!    cannot make region building or the macOS hit test expensive.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, SkinWindow, WindowLayout};

use super::silhouette::{boxes, placed};
use super::{silhouette, Mask, ShapeError, SCALE_ONE};
use crate::images::decode_png;

/// The most rectangles a silhouette may take. The packs' real counts at
/// [`MAX_WINDOW_WIDTH`] × [`MAX_WINDOW_HEIGHT`] are at most a few hundred
/// (the corner curves give a rectangle per distinct scanline, the side
/// edges one run per row group; the shape tests print them), so 4096 is
/// ten times the worst built-in and still a trivially small Win32 region
/// or GDK `cairo::Region`; the count only grows with the window when a
/// side or middle piece's mask varies along its repeat, which this cap
/// bounds (research §6.4).
pub const MAX_RECTS: usize = 4096;
/// The largest window the cap is checked at, logical px (an 8K display).
pub const MAX_WINDOW_WIDTH: u32 = 7680;
pub const MAX_WINDOW_HEIGHT: u32 = 4320;

fn name(window: SkinWindow) -> String {
    format!("{window:?}").to_ascii_lowercase()
}

fn bad(field: &str, expected: &str) -> SkinError {
    SkinError::new(
        field,
        SkinRule::BadValue {
            field: field.to_owned(),
            expected: expected.to_owned(),
        },
    )
}

fn masks_match(
    window: SkinWindow,
    layout: &WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<(), SkinError> {
    for (i, t) in layout.tiles.iter().enumerate() {
        let field = format!("windows.{}.tiles.{i}", name(window));
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
            return Err(bad(&field, "the image's width and height"));
        }
        let mask = Mask::from_hex(&t.mask, img.width(), img.height());
        if mask.as_ref() != Some(&Mask::of(&img)) {
            return Err(bad(&field, "the image's 1-bit alpha mask"));
        }
    }
    Ok(())
}

fn shape_err(window: SkinWindow, e: &ShapeError) -> SkinError {
    let w = name(window);
    match *e {
        ShapeError::Mask { tile } => bad(&format!("windows.{w}.tiles.{tile}"), "a 1-bit mask"),
        ShapeError::TooManyRects { count } => SkinError::new(
            format!("windows.{w}").as_str(),
            SkinRule::ShapeTooManyRects {
                window: w.clone(),
                count: count as u64,
                max: MAX_RECTS as u64,
            },
        ),
    }
}

/// Rule 2 and 3 at `width × height`.
fn closed(
    window: SkinWindow,
    layout: &WindowLayout,
    width: u32,
    height: u32,
) -> Result<(), SkinError> {
    let w = name(window);
    let (_, c) = boxes(layout, width, height);
    for (i, (x, y, tw, th)) in placed(layout, width, height) {
        if layout.tiles[i].overhang {
            continue;
        }
        if x < c.0 + c.2 && c.0 < x + tw && y < c.1 + c.3 && c.1 < y + th && c.2 > 0 && c.3 > 0 {
            return Err(SkinError::new(
                format!("windows.{w}.tiles.{i}").as_str(),
                SkinRule::ShapeIntrudes {
                    window: w,
                    tile: i as u64,
                },
            ));
        }
    }
    let rects = silhouette(layout, width, height, SCALE_ONE).map_err(|e| shape_err(window, &e))?;
    let (ww, hh) = (width as usize, height as usize);
    let mut opaque = vec![false; ww * hh];
    for r in rects {
        for y in r.y..r.y + r.height {
            let row = y as usize * ww;
            opaque[row + r.x as usize..row + (r.x + r.width) as usize].fill(true);
        }
    }
    // Flood the transparent pixels from the border; any left is a hole.
    let mut seen = opaque.clone();
    let mut stack: Vec<usize> = (0..ww * hh)
        .filter(|&i| {
            let (x, y) = (i % ww, i / ww);
            (x == 0 || y == 0 || x + 1 == ww || y + 1 == hh) && !opaque[i]
        })
        .collect();
    for &i in &stack {
        seen[i] = true;
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % ww, i / ww);
        let near = [
            (x > 0).then(|| i - 1),
            (x + 1 < ww).then(|| i + 1),
            (y > 0).then(|| i - ww),
            (y + 1 < hh).then(|| i + ww),
        ];
        for n in near.into_iter().flatten() {
            if !seen[n] {
                seen[n] = true;
                stack.push(n);
            }
        }
    }
    if let Some(i) = seen.iter().position(|s| !s) {
        return Err(SkinError::new(
            format!("windows.{w}").as_str(),
            SkinRule::ShapeHole {
                window: w,
                x: u32::try_from(i % ww).unwrap_or(u32::MAX),
                y: u32::try_from(i / ww).unwrap_or(u32::MAX),
            },
        ));
    }
    Ok(())
}

/// Refuses a manifest whose frame layouts break the rules above;
/// `images` are the package's canonical images.
///
/// # Errors
/// The first rule a layout breaks.
pub fn check(manifest: &SkinManifest, images: &BTreeMap<String, Vec<u8>>) -> Result<(), SkinError> {
    for (&window, layout) in &manifest.windows {
        masks_match(window, layout, images)?;
        let o = layout.outsets;
        let base_w = u32::from(layout.min_width) + u32::from(o.left) + u32::from(o.right);
        let base_h = u32::from(layout.min_height) + u32::from(o.top) + u32::from(o.bottom);
        let span = |f: fn(&rekindle_types::dto::skin::FrameTile) -> u16| {
            layout
                .tiles
                .iter()
                .map(|t| u32::from(f(t)))
                .max()
                .unwrap_or(0)
        };
        // The smallest frame, and one with every repeat laid twice over.
        closed(window, layout, base_w, base_h)?;
        closed(
            window,
            layout,
            base_w + 2 * span(|t| t.width) + 1,
            base_h + 2 * span(|t| t.height) + 1,
        )?;
        silhouette(layout, MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT, SCALE_ONE)
            .map_err(|e| shape_err(window, &e))?;
    }
    Ok(())
}
