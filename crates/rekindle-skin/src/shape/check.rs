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
//!    content's interactive layer and the content stays opaque under it.
//!    A fill tile (`FrameTile::fill`, the pack's centre art) is allowed
//!    only in the band between a side piece and the content (plan SK0-L1,
//!    lead decision for Space Cowboy): every copy wholly inside the frame
//!    rectangle, beside the content and no taller than it, never over it,
//!    not an overhang or a decoration, and its art wholly opaque;
//! 3. the content rectangle is fully opaque: an overhang tile may lie over
//!    it only with opaque pixels, so nothing behind the window ever shows
//!    through the content. Transparent pixels in the frame band, enclosed
//!    or not, are allowed (Xfire's 1-bit regions had them; lead decision
//!    (a), plan SK0 S1);
//! 4. at most [`MAX_RECTS`] rectangles at the largest window, so a skin
//!    cannot make region building or the macOS hit test expensive (this is
//!    what bounds the frame band's transparency);
//! 5. at most [`MAX_FRAME_TILES`] tiles, the slots the window draws them in
//!    (plan SK0 S2);
//! 6. a header region (plan SK0-L1) is the main window's only, placed once
//!    on each axis, not empty, inside the window, outside the content
//!    rectangle and wholly over the frame's opaque pixels, so the shaper
//!    never cuts the profile header and no other app shows through it.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, SkinWindow, WindowLayout};

use rekindle_types::dto::skin::TileAxis;

use super::silhouette::{boxes, placed, regions};
use super::{of_layout, silhouette, Mask, ShapeError, MAX_FRAME_TILES, SCALE_ONE};

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

/// Rule 2's fill band at `width × height`: each fill tile's copies lie
/// beside the content rectangle inside the frame's, and its art is opaque.
fn fill_band(
    window: SkinWindow,
    layout: &WindowLayout,
    masks: &[Mask],
    width: u32,
    height: u32,
) -> Result<(), SkinError> {
    let w = name(window);
    let (f, c) = boxes(layout, width, height);
    for (i, (x, y, tw, th)) in placed(layout, width, height) {
        let t = &layout.tiles[i];
        if !t.fill {
            continue;
        }
        let opaque = (0..u32::from(t.height))
            .all(|my| (0..u32::from(t.width)).all(|mx| masks[i].get(mx, my)));
        let in_frame = x >= f.0 && y >= f.1 && x + tw <= f.0 + f.2 && y + th <= f.1 + f.3;
        let beside = (x + tw <= c.0 || x >= c.0 + c.2) && y >= c.1 && y + th <= c.1 + c.3;
        if t.overhang || t.decoration.is_some() || !opaque || !in_frame || !beside {
            return Err(bad(
                &format!("windows.{w}.tiles.{i}"),
                "a centre fill wholly opaque in the band between a side piece and the content",
            ));
        }
    }
    Ok(())
}

/// Rules 2 to 4 at `width × height`.
fn closed(
    window: SkinWindow,
    layout: &WindowLayout,
    masks: &[Mask],
    width: u32,
    height: u32,
) -> Result<(), SkinError> {
    fill_band(window, layout, masks, width, height)?;
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

/// Rule 6 at `width × height`.
fn header(
    window: SkinWindow,
    layout: &WindowLayout,
    masks: &[Mask],
    width: u32,
    height: u32,
) -> Result<(), SkinError> {
    let Some(region) = layout.header else {
        return Ok(());
    };
    let field = format!("windows.{}.header", name(window));
    if window != SkinWindow::Main {
        return Err(bad(&field, "no header region (main window only)"));
    }
    if matches!(region.x, TileAxis::Repeat { .. }) || matches!(region.y, TileAxis::Repeat { .. }) {
        return Err(bad(&field, "a region placed once on each axis"));
    }
    if region.width == 0 || region.height == 0 {
        return Err(bad(&field, "a region at least 1 px each way"));
    }
    let (content, Some((left, top, wide, tall))) = regions(layout, width, height) else {
        return Ok(());
    };
    if left < 0 || top < 0 || left + wide > i64::from(width) || top + tall > i64::from(height) {
        return Err(bad(&field, "a region inside the window"));
    }
    let (cx, cy, cw, ch) = content;
    if left < cx + cw && cx < left + wide && top < cy + ch && cy < top + tall {
        return Err(bad(&field, "a region outside the content rectangle"));
    }
    let tiles = placed(layout, width, height);
    for py in top..top + tall {
        for px in left..left + wide {
            let opaque = tiles.iter().any(|&(i, (tx, ty, tw, th))| {
                px >= tx
                    && py >= ty
                    && px < tx + tw
                    && py < ty + th
                    && u32::try_from(px - tx)
                        .ok()
                        .zip(u32::try_from(py - ty).ok())
                        .is_some_and(|(mx, my)| masks[i].get(mx, my))
            });
            if !opaque {
                return Err(bad(&field, "a region over the frame's opaque pixels"));
            }
        }
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
        // Rule 5: no more tiles than the window draws (`SkinFrame`'s slots).
        if layout.tiles.len() > MAX_FRAME_TILES {
            return Err(bad(
                &format!("windows.{}.tiles", name(window)),
                "at most 32 tiles",
            ));
        }
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
        let (big_w, big_h) = (
            base_w + 2 * span(|t| t.width) + 1,
            base_h + 2 * span(|t| t.height) + 1,
        );
        closed(window, layout, &masks, big_w, big_h)?;
        header(window, layout, &masks, base_w, base_h)?;
        header(window, layout, &masks, big_w, big_h)?;
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
