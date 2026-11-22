//! The cover of a glyph label (plan SK3, the contract with the shell): an
//! icon-strip Communities clone ([`super::place`]) copies the last tab's
//! art, icon and all (Symbiosis's `ButtonVideosTab`, a video camera's
//! clapperboard). Its label carries `cover: [x, y, w, h]`, the box of that
//! copied icon in the art, so the shell fills the box with the button's
//! ground and draws the Communities glyph there instead.
//!
//! The icon is what stands off the button's ground and does not reach the
//! art's edge:
//!
//! - each row's ground is the commonest colour of its outer quarters (an
//!   icon button centres its icon, so the flanks are ground even where
//!   the icon is wider than the plate it sits on: Symbiosis's 18 px
//!   clapper body against 16 px of plate in its row), so a plate graded
//!   top to bottom (Symbiosis's `#59534d` to `#3c3b3a`) is followed row
//!   by row;
//! - a pixel differs when a channel is more than [`ICON_TOLERANCE`] from its
//!   row's ground (gloss and anti-aliasing stay ground);
//! - the differing pixels 4-connected to the art's edge are the frame (the
//!   window ground around a rounded plate, the plate's dark rim) and are
//!   dropped; the box of the rest is the icon.
//!
//! On Symbiosis's 46 x 48 `node-buttonvideostab.png` this is
//! `[14, 15, 18, 17]`, the clapperboard (its stick from row 15, its body
//! to row 31, x 14-31), the same box at every tolerance from 16 to 32 and
//! within a pixel on the hover and down art. Art with no such icon (the
//! box empty, or the icon touching the frame so the box would be the
//! whole art) serves no cover, and the glyph is drawn over the plain art.

use std::collections::HashMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_tree::{NodePiece, NodeSpan, PieceSlot, SkinNode};

use crate::package::Package;

/// The largest channel difference from a row's ground that is still
/// ground: a tenth of the range, inside the band (16-32) where
/// Symbiosis's icon box holds still.
pub(crate) const ICON_TOLERANCE: u8 = 24;

/// The box of the icon in `image`, or `None` when it has none (see the
/// module doc).
pub(crate) fn icon_box(image: &RgbaImage) -> Option<[u16; 4]> {
    let width = usize::try_from(image.width()).ok()?;
    let height = usize::try_from(image.height()).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let differs = differing(image, width);
    let frame = edge_connected(&differs, width, height);
    // [left, top, right, bottom], inclusive.
    let mut found: Option<[usize; 4]> = None;
    for (at, _) in differs
        .iter()
        .zip(&frame)
        .enumerate()
        .filter(|(_, (d, f))| **d && !**f)
    {
        let (col, row) = (at % width, at / width);
        found = Some(found.map_or([col, row, col, row], |[l, t, r, b]| {
            [l.min(col), t.min(row), r.max(col), b.max(row)]
        }));
    }
    let [left, top, right, bottom] = found?;
    if left == 0 && top == 0 && right + 1 == width && bottom + 1 == height {
        return None;
    }
    let side = |v: usize| u16::try_from(v).ok();
    Some([
        side(left)?,
        side(top)?,
        side(right - left + 1)?,
        side(bottom - top + 1)?,
    ])
}

/// Each pixel's difference from its row's ground (row-major).
fn differing(image: &RgbaImage, width: usize) -> Vec<bool> {
    let quarter = (width / 4).max(1);
    let mut out = Vec::with_capacity(width * image.rows().len());
    for pixels in image.rows() {
        let row: Vec<[u8; 4]> = pixels.map(|p| p.0).collect();
        let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
        let flanks = row
            .iter()
            .take(quarter)
            .chain(row.iter().rev().take(quarter));
        for colour in flanks {
            *counts.entry(*colour).or_default() += 1;
        }
        let ground = counts
            .into_iter()
            .max_by_key(|&(colour, count)| (count, colour))
            .map_or([0; 4], |(colour, _)| colour);
        out.extend(row.iter().map(|colour| {
            colour
                .iter()
                .zip(ground)
                .any(|(a, b)| a.abs_diff(b) > ICON_TOLERANCE)
        }));
    }
    out
}

/// The differing pixels 4-connected to the image's edge.
fn edge_connected(differs: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut seen = vec![false; w * h];
    let mut stack: Vec<(usize, usize)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| x == 0 || y == 0 || x + 1 == w || y + 1 == h)
        .collect();
    while let Some((x, y)) = stack.pop() {
        let i = y * w + x;
        if seen[i] || !differs[i] {
            continue;
        }
        seen[i] = true;
        if x > 0 {
            stack.push((x - 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if x + 1 < w {
            stack.push((x + 1, y));
        }
        if y + 1 < h {
            stack.push((x, y + 1));
        }
    }
    seen
}

/// The node's one whole picture, its own Normal Middle or a child's, and
/// where it sits in the node's `w x h` box.
fn picture(n: &SkinNode, w: i32, h: i32) -> Option<(&NodePiece, i32, i32)> {
    let own = n
        .art
        .as_ref()
        .filter(|a| a.normal.len() == 1)
        .and_then(|a| a.normal.get(&PieceSlot::Middle));
    if let Some(p) = own {
        return Some((p, 0, 0));
    }
    n.children.iter().find_map(|c| {
        let (cx, cw) = c.x.place(w);
        let (cy, ch) = c.y.place(h);
        picture(c, cw, ch).map(|(p, x, y)| (p, cx + x, cy + y))
    })
}

/// The cover of a glyph label on `n` (a fixed box), from its picture.
fn cover_of(node: &SkinNode, package: &Package<'_>) -> Result<Option<[u16; 4]>, SkinError> {
    let (NodeSpan::Fixed { size: width, .. }, NodeSpan::Fixed { size: height, .. }) =
        (node.x, node.y)
    else {
        return Ok(None);
    };
    let Some((piece, dx, dy)) = picture(node, i32::from(width), i32::from(height)) else {
        return Ok(None);
    };
    let bytes = package.get(&piece.image).ok_or_else(|| {
        SkinError::new(
            piece.image.as_str(),
            SkinRule::MissingEntry {
                field: format!("{}.art.normal.middle", node.id),
            },
        )
    })?;
    let frames = crate::regions::frames(&piece.image, bytes)?;
    let Some([left, top, across, down]) = frames.first().and_then(icon_box) else {
        return Ok(None);
    };
    let shift = |v: u16, by: i32| u16::try_from(i32::from(v) + by).ok();
    Ok(shift(left, dx)
        .zip(shift(top, dy))
        .map(|(left, top)| [left, top, across, down]))
}

fn walk(nodes: &mut [SkinNode], package: &Package<'_>) -> Result<(), SkinError> {
    for n in nodes {
        if n.label.as_ref().is_some_and(|l| l.glyph.is_some()) {
            let cover = cover_of(n, package)?;
            if let Some(l) = n.label.as_mut() {
                l.cover = cover;
            }
        }
        walk(&mut n.children, package)?;
    }
    Ok(())
}

/// Sets the cover of every glyph label in `layout`'s tree and popups from
/// the package's art. Worded labels keep the cover [`super::place`] gave.
///
/// # Errors
/// A glyph label's picture missing from the package or not decoding.
pub fn glyphs(layout: &mut WindowLayout, package: &Package<'_>) -> Result<(), SkinError> {
    if let Some(tree) = layout.tree.as_mut() {
        walk(&mut tree.root, package)?;
    }
    for popup in layout.popups.values_mut() {
        walk(&mut popup.tree.root, package)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rekindle_image::{Rgba, RgbaImage};

    use super::icon_box;

    /// A 46 x 48 window ground (`#151515`) with a plate graded top to
    /// bottom inside a dark rim, as Symbiosis's tab art.
    fn plate() -> RgbaImage {
        RgbaImage::from_fn(46, 48, |x, y| {
            if !(2..44).contains(&x) || !(9..38).contains(&y) {
                Rgba([0x15, 0x15, 0x15, 255])
            } else if x == 2 || x == 43 || y == 9 || y == 37 {
                Rgba([9, 9, 9, 255])
            } else {
                let v = u8::try_from(0x59 - (y - 10) / 2).unwrap();
                Rgba([v, v - 4, v - 8, 255])
            }
        })
    }

    #[test]
    fn the_icon_on_a_graded_plate_is_its_box() {
        let mut art = plate();
        // Wider than the plate left beside it in its rows.
        for y in 20..26 {
            for x in 13..33 {
                art.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        assert_eq!(icon_box(&art), Some([13, 20, 20, 6]));
    }

    #[test]
    fn a_plain_plate_or_an_icon_on_the_frame_has_no_cover() {
        assert_eq!(icon_box(&plate()), None);
        let mut art = plate();
        for x in 0..46 {
            art.put_pixel(x, 24, Rgba([255, 255, 255, 255]));
        }
        assert_eq!(icon_box(&art), None);
    }
}
