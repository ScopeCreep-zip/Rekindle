//! The content region from a pack's own views (plan SK0-L1, lead
//! decision: "the content box is the pack's content region exactly").
//!
//! Xfire lays the buddy list's views in one tile stretched both ways (the
//! XML era's `TabbedControl`, Symbiosis's `XfireView`; the INI era's
//! `[MainWindow]` window `Name=TabbedControl`), `X` px from the frame's
//! left and `IndentRight` px from its right. Where those views reach over
//! the frame's side pieces (Total Chaos: views 50 px in, sides 59 px;
//! Onyx: 8 px in), the sides are laid only to the views' edge: their inner
//! columns lie under the pack's views and are never shown, so the content
//! rectangle, whose side insets are the side pieces' widths, is the
//! views' own. A side already narrower than the views' inset is kept
//! whole (the content never grows past a frame piece into a band no tile
//! covers).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};

use super::ImportError;
use crate::shape::{self, u16_of};

/// The first `width` columns of `img` (all of it when it is narrower).
#[must_use]
pub fn first_columns(img: &RgbaImage, width: u32) -> RgbaImage {
    let w = width.min(img.width());
    RgbaImage::from_fn(w, img.height(), |x, y| *img.get_pixel(x, y))
}

/// The last `width` columns of `img` (all of it when it is narrower).
#[must_use]
pub fn last_columns(img: &RgbaImage, width: u32) -> RgbaImage {
    let w = width.min(img.width());
    let skip = img.width() - w;
    RgbaImage::from_fn(w, img.height(), |x, y| *img.get_pixel(x + skip, y))
}

/// Lays the side pieces `left`/`right` only to the views' edge, the views
/// `insets` (left, right) px inside the frame: a side wider than its inset
/// keeps its outer columns; a zero inset or one at least the side's width
/// leaves it whole.
pub fn to_views_edge(left: &mut RgbaImage, right: &mut RgbaImage, insets: (u32, u32)) {
    let (l, r) = insets;
    if l > 0 && l < left.width() {
        *left = first_columns(left, l);
    }
    if r > 0 && r < right.width() {
        *right = last_columns(right, r);
    }
}

/// The first `height` rows of `img` (all of it when it is shorter).
#[must_use]
pub fn first_rows(img: &RgbaImage, height: u32) -> RgbaImage {
    let h = height.min(img.height());
    RgbaImage::from_fn(img.width(), h, |x, y| *img.get_pixel(x, y))
}

/// The last `height` rows of `img` (all of it when it is shorter).
#[must_use]
pub fn last_rows(img: &RgbaImage, height: u32) -> RgbaImage {
    let h = height.min(img.height());
    let skip = img.height() - h;
    RgbaImage::from_fn(img.width(), h, |x, y| *img.get_pixel(x, y + skip))
}

/// Lays the top and bottom rows of the nine `pieces` (in
/// [`crate::import::xfire_xml::bitmap::NINE`] order) only to the views'
/// edge, the views `insets` (top, bottom) px inside the frame, as
/// [`to_views_edge`] does the sides: a piece taller than its inset keeps
/// its outer rows (Alloy: top corners 252 px, views 157 px down; Combat:
/// 214 and 194), whose inner rows lie under the views and are never shown;
/// a zero inset or one at least the piece's height leaves it whole.
pub fn to_views_rows(pieces: &mut [RgbaImage; 9], insets: (u32, u32)) {
    let (top, bottom) = insets;
    for (i, piece) in pieces.iter_mut().enumerate() {
        match i {
            0..=2 if top > 0 && top < piece.height() => *piece = first_rows(piece, top),
            6..=8 if bottom > 0 && bottom < piece.height() => {
                *piece = last_rows(piece, bottom);
            }
            _ => {}
        }
    }
}

/// Widens `built`'s content to the views `insets` (left, right) where a
/// side piece is narrower than its inset (Space Cowboy: sides 16/20,
/// views 27/30), painting the band between the side piece and the views
/// with the pack's `centre` fill (lead decision, plan SK0-L1: the original
/// shows its background there), one [`FrameTile::fill`] tile per side laid
/// down the content's height. The centre art must be present and wholly
/// opaque (the band is then never see-through, rule 2 of
/// [`crate::shape::check`]); without it no band is laid and the content
/// keeps the side's width (the content never grows into a band no tile
/// covers).
///
/// # Errors
/// A band too large for a tile, or one that does not encode.
///
/// [`FrameTile::fill`]: rekindle_types::dto::skin::FrameTile::fill
pub fn fill_band(
    window: SkinWindow,
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
    centre: &RgbaImage,
    insets: (u32, u32),
) -> Result<(), ImportError> {
    let present = centre.width() > 0
        && centre.height() > 0
        && centre.pixels().all(|p| p[3] >= shape::ALPHA_OPAQUE);
    if !present {
        return Ok(());
    }
    let file = shape::tile_path(window, "frame");
    let content = built.0.content;
    let y = TileAxis::Repeat {
        start: content.top,
        end: content.bottom,
    };
    for (left, inset) in [(true, insets.0), (false, insets.1)] {
        let edge = if left { content.left } else { content.right };
        if inset <= u32::from(edge) {
            continue;
        }
        let band = inset - u32::from(edge);
        let img = RgbaImage::from_fn(band, centre.height(), |x, y| {
            *centre.get_pixel(x % centre.width(), y)
        });
        let (name, x) = if left {
            ("views-band-left", TileAxis::Start { offset: edge })
        } else {
            ("views-band-right", TileAxis::End { offset: edge })
        };
        let Some((mut t, png)) = shape::tile(window, name, &img, (x, y), false)? else {
            continue;
        };
        t.fill = true;
        built.1.insert(t.image.clone(), png);
        built.0.tiles.push(t);
        let inset = u16_of(&file, inset)?;
        if left {
            built.0.content.left = inset;
        } else {
            built.0.content.right = inset;
        }
    }
    let c = built.0.content;
    built.0.min_width = built.0.min_width.max(c.left.saturating_add(c.right));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn striped(w: u32) -> RgbaImage {
        RgbaImage::from_fn(w, 2, |x, _| {
            rekindle_image::Rgba([u8::try_from(x).unwrap(), 0, 0, 255])
        })
    }

    #[test]
    fn sides_wider_than_the_views_inset_keep_their_outer_columns() {
        let (mut l, mut r) = (striped(59), striped(59));
        to_views_edge(&mut l, &mut r, (50, 8));
        assert_eq!((l.width(), r.width()), (50, 8));
        assert_eq!(l.get_pixel(0, 0)[0], 0);
        assert_eq!(r.get_pixel(7, 0)[0], 58);
    }

    /// Plan SK0-L1 (Alloy, Combat, XGlow): rows taller than the views'
    /// inset keep their outer rows, the middle row and shorter pieces stay
    /// whole, and the built content is the views' rectangle.
    #[test]
    fn rows_taller_than_the_views_inset_keep_their_outer_rows() {
        let rows = |h: u32| {
            RgbaImage::from_fn(4, h, |_, y| {
                rekindle_image::Rgba([u8::try_from(y).unwrap(), 0, 0, 255])
            })
        };
        let mut nine: [RgbaImage; 9] = std::array::from_fn(|i| match i {
            0..=2 => rows(30),
            6..=8 => rows(20),
            _ => rows(5),
        });
        nine[1] = rows(10);
        to_views_rows(&mut nine, (16, 7));
        let heights: Vec<u32> = nine.iter().map(RgbaImage::height).collect();
        assert_eq!(heights, vec![16, 10, 16, 5, 5, 5, 7, 7, 7]);
        assert_eq!(nine[0].get_pixel(0, 15)[0], 15);
        assert_eq!(nine[8].get_pixel(0, 0)[0], 13);
        let built = shape::from_nine(SkinWindow::Main, &nine).unwrap();
        assert_eq!((built.0.content.top, built.0.content.bottom), (16, 7));
    }

    #[test]
    fn sides_narrower_than_the_views_inset_or_no_inset_stay_whole() {
        let (mut l, mut r) = (striped(16), striped(20));
        to_views_edge(&mut l, &mut r, (27, 0));
        assert_eq!((l.width(), r.width()), (16, 20));
    }

    /// Plan SK0-L1 (Space Cowboy, sides narrower than the views): the
    /// content is the views' rectangle and the band is the centre art; a
    /// centre with a transparent pixel lays no band.
    #[test]
    fn a_narrow_sides_band_to_the_views_is_the_centre_fill() {
        let px = |a| rekindle_image::Rgba([9, 9, 9, a]);
        let nine: [RgbaImage; 9] = std::array::from_fn(|i| {
            if i == 4 {
                RgbaImage::from_fn(3, 2, |x, _| {
                    rekindle_image::Rgba([u8::try_from(x).unwrap(), 0, 0, 255])
                })
            } else {
                RgbaImage::from_pixel(4, 4, px(255))
            }
        });
        let mut built = shape::from_nine(SkinWindow::Main, &nine).unwrap();
        fill_band(SkinWindow::Main, &mut built, &nine[4], (9, 4)).unwrap();
        assert_eq!((built.0.content.left, built.0.content.right), (9, 4));
        assert!(built.0.min_width >= 13);
        let fills: Vec<_> = built
            .0
            .tiles
            .iter()
            .filter(|t| t.fill)
            .map(|t| (t.width, t.height, t.x, t.y, t.overhang))
            .collect();
        let y = TileAxis::Repeat { start: 4, end: 4 };
        assert_eq!(fills, vec![(5, 2, TileAxis::Start { offset: 4 }, y, false)]);
        let mut clear = shape::from_nine(SkinWindow::Main, &nine).unwrap();
        let mut centre = nine[4].clone();
        centre.put_pixel(1, 1, px(0));
        fill_band(SkinWindow::Main, &mut clear, &centre, (9, 4)).unwrap();
        assert_eq!(clear.0.content.left, 4);
        assert!(!clear.0.tiles.iter().any(|t| t.fill));
    }
}
