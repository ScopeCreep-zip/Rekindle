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

use rekindle_image::RgbaImage;

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

    #[test]
    fn sides_narrower_than_the_views_inset_or_no_inset_stay_whole() {
        let (mut l, mut r) = (striped(16), striped(20));
        to_views_edge(&mut l, &mut r, (27, 0));
        assert_eq!((l.width(), r.width()), (16, 20));
    }
}
