//! The helper's refusals of a frame layout (plan SK0 S1, S4; research §6).
//!
//! A shaped window shows, and passes clicks to, whatever lies behind its
//! transparent pixels, so a hostile skin could frame another app's button
//! as Rekindle's (UI redressing; threat model). The rules:
//! 1. each tile's mask is its image's alpha (≥ [`super::ALPHA_OPAQUE`]),
//!    derived from the image, never declared: the shape is what is drawn;
//! 2. no frame tile reaches into the content rectangle (security surfaces
//!    live there, `@layer sec`); a decoration (a pack tile above the frame
//!    band, `FrameTile::decoration`) may, because it draws beneath the
//!    content's interactive layer and the content stays opaque under it;
//! 3. the content rectangle is fully opaque: an overhang tile may lie over
//!    it only with opaque pixels, so nothing behind the window ever shows
//!    through the content. Transparent pixels in the frame band, enclosed
//!    or not, are allowed (Xfire's 1-bit regions had them; lead decision
//!    (a), plan SK0 S1);
//! 4. at most [`MAX_RECTS`] rectangles at the largest window, so a skin
//!    cannot make region building or the macOS hit test expensive (this is
//!    what bounds the frame band's transparency).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, SkinWindow, WindowLayout};

use super::silhouette::{boxes, placed};
use super::{of_layout, silhouette, Mask, ShapeError, SCALE_ONE};

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

/// Rules 2 to 4 at `width × height`.
fn closed(
    window: SkinWindow,
    layout: &WindowLayout,
    masks: &[Mask],
    width: u32,
    height: u32,
) -> Result<(), SkinError> {
    let w = name(window);
    let (_, c) = boxes(layout, width, height);
    for (i, (x, y, tw, th)) in placed(layout, width, height) {
        let (x0, x1) = (x.max(c.0), (x + tw).min(c.0 + c.2));
        let (y0, y1) = (y.max(c.1), (y + th).min(c.1 + c.3));
        // A decoration draws beneath the content's interactive layer and
        // the content stays opaque under it (lead decision, plan SK0 S1c).
        if x0 >= x1 || y0 >= y1 || layout.tiles[i].decoration.is_some() {
            continue;
        }
        if !layout.tiles[i].overhang {
            return Err(SkinError::new(
                format!("windows.{w}.tiles.{i}").as_str(),
                SkinRule::ShapeIntrudes {
                    window: w,
                    tile: i as u64,
                },
            ));
        }
        for py in y0..y1 {
            for px in x0..x1 {
                let (mx, my) = (px - x, py - y);
                let opaque = u32::try_from(mx)
                    .ok()
                    .zip(u32::try_from(my).ok())
                    .is_some_and(|(mx, my)| masks[i].get(mx, my));
                if !opaque {
                    return Err(SkinError::new(
                        format!("windows.{w}.tiles.{i}").as_str(),
                        SkinRule::ShapeContentTransparent {
                            window: w,
                            x: u32::try_from(px).unwrap_or(u32::MAX),
                            y: u32::try_from(py).unwrap_or(u32::MAX),
                        },
                    ));
                }
            }
        }
    }
    silhouette(layout, masks, width, height, SCALE_ONE).map_err(|e| shape_err(window, &e))?;
    Ok(())
}

/// Refuses a manifest whose frame layouts break the rules above;
/// `images` are the package's canonical images.
///
/// # Errors
/// The first rule a layout breaks.
pub fn check(manifest: &SkinManifest, images: &BTreeMap<String, Vec<u8>>) -> Result<(), SkinError> {
    for (&window, layout) in &manifest.windows {
        let masks = of_layout(&format!("windows.{}", name(window)), layout, images)?;
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
        closed(window, layout, &masks, base_w, base_h)?;
        closed(
            window,
            layout,
            &masks,
            base_w + 2 * span(|t| t.width) + 1,
            base_h + 2 * span(|t| t.height) + 1,
        )?;
        silhouette(
            layout,
            &masks,
            MAX_WINDOW_WIDTH,
            MAX_WINDOW_HEIGHT,
            SCALE_ONE,
        )
        .map_err(|e| shape_err(window, &e))?;
    }
    Ok(())
}
