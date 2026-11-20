//! Silhouettes and the helper's refusals (plan SK0 S1).

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{SkinManifest, SkinRule, SkinWindow, TileAxis, WindowLayout};

use super::*;
use crate::{builtin, images, manifest, package};

const ON: Rgba<u8> = Rgba([10, 20, 30, 255]);
const OFF: Rgba<u8> = Rgba([0, 0, 0, 0]);

/// A `size` corner piece, cut where `x + y < cut` (top left), mirrored by
/// `flip` (x, y).
fn corner(size: u32, cut: u32, flip: (bool, bool)) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        let x = if flip.0 { size - 1 - x } else { x };
        let y = if flip.1 { size - 1 - y } else { y };
        if x + y < cut {
            OFF
        } else {
            ON
        }
    })
}

fn solid(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_pixel(w, h, ON)
}

fn nine() -> [RgbaImage; 9] {
    [
        corner(4, 2, (false, false)),
        solid(2, 4),
        corner(4, 2, (true, false)),
        solid(4, 2),
        RgbaImage::from_pixel(1, 1, OFF),
        solid(4, 2),
        corner(4, 2, (false, true)),
        solid(2, 4),
        corner(4, 2, (true, true)),
    ]
}

fn r(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn masks(layout: &WindowLayout, files: &BTreeMap<String, Vec<u8>>) -> Vec<Mask> {
    of_layout("windows.main", layout, files).unwrap()
}

#[test]
fn corners_cut_and_middles_repeat() {
    let (layout, files) = from_nine(SkinWindow::Main, &nine()).unwrap();
    let m = masks(&layout, &files);
    assert_eq!(layout.tiles.len(), 8, "the centre is the content ground");
    assert_eq!(files.len(), 8);
    let want = vec![
        r(2, 0, 6, 1),
        r(1, 1, 8, 1),
        r(0, 2, 10, 6),
        r(1, 8, 8, 1),
        r(2, 9, 6, 1),
    ];
    assert_eq!(silhouette(&layout, &m, 10, 10, SCALE_ONE).unwrap(), want);
    let doubled: Vec<Rect> = want
        .iter()
        .map(|q| r(q.x * 2, q.y * 2, q.width * 2, q.height * 2))
        .collect();
    assert_eq!(
        silhouette(&layout, &m, 10, 10, 2 * SCALE_ONE).unwrap(),
        doubled
    );
    // Determinism: the same layout gives the same rectangles.
    assert_eq!(
        silhouette(&layout, &m, 37, 23, 1250).unwrap(),
        silhouette(&layout, &m, 37, 23, 1250).unwrap()
    );
}

#[test]
fn a_mask_is_the_images_alpha() {
    let img = corner(9, 4, (false, false));
    let m = Mask::of(&img);
    assert!(!m.get(0, 0) && !m.get(3, 0) && m.get(4, 0) && m.get(8, 8));
    assert_eq!(m.runs(1), vec![(3, 9)]);
}

fn base() -> SkinManifest {
    let (_, b) = builtin::find(builtin::DEFAULT).unwrap();
    manifest::parse(&package::read(b.bytes).unwrap()).unwrap()
}

fn checked(pieces: &[RgbaImage; 9], edit: impl Fn(&mut WindowLayout)) -> Result<(), SkinRule> {
    checked_with(pieces, |l, _| edit(l))
}

fn checked_with(
    pieces: &[RgbaImage; 9],
    edit: impl Fn(&mut WindowLayout, &mut BTreeMap<String, Vec<u8>>),
) -> Result<(), SkinRule> {
    let (mut layout, mut files) = from_nine(SkinWindow::Chat, pieces).unwrap();
    edit(&mut layout, &mut files);
    let mut m = base();
    m.windows = BTreeMap::from([(SkinWindow::Chat, layout)]);
    check(&m, &files).map_err(|e| e.rule)
}

#[test]
fn a_well_formed_frame_passes() {
    assert_eq!(checked(&nine(), |_| {}), Ok(()));
}

#[test]
fn the_content_sides_are_the_edge_pieces_not_the_corners() {
    // xfire-classic: 117 px bottom corners hold console art in the bottom
    // row, the side edges 7 and 6 px (plan SK0 S2 finding).
    let mut pieces = nine();
    pieces[3] = solid(7, 2);
    pieces[5] = solid(6, 2);
    pieces[6] = solid(117, 9);
    pieces[8] = solid(115, 9);
    let (layout, _) = from_nine(SkinWindow::Main, &pieces).unwrap();
    let c = layout.content;
    assert_eq!((c.top, c.right, c.bottom, c.left), (4, 6, 9, 7));
    assert_eq!(checked(&pieces, |_| {}), Ok(()));
}

#[test]
fn a_layout_past_the_windows_tile_slots_is_refused() {
    // The window draws a tile per `SkinFrame` slot (plan SK0 S2).
    let fill = |l: &mut WindowLayout| {
        let first = l.tiles[0].clone();
        l.tiles.resize(MAX_FRAME_TILES, first);
    };
    assert_eq!(checked(&nine(), fill), Ok(()));
    let got = checked(&nine(), |l| {
        fill(l);
        l.tiles.push(l.tiles[0].clone());
    });
    assert_eq!(
        got,
        Err(SkinRule::BadValue {
            field: "windows.chat.tiles".into(),
            expected: "at most 32 tiles".into(),
        })
    );
}

#[test]
fn an_enclosed_gap_in_the_frame_band_is_allowed() {
    // Lead decision (a): transparency in the frame band, enclosed or not,
    // is Xfire's (gaps under short top pieces in Alloy and Combat).
    let mut p = nine();
    p[0].put_pixel(2, 2, OFF);
    assert_eq!(checked(&p, |_| {}), Ok(()));
}

#[test]
fn a_transparent_pixel_inside_the_content_rectangle_is_refused() {
    // An overhang laid centred on the window, over the content rectangle,
    // with one transparent pixel there.
    let mut plaque = solid(4, 4);
    plaque.put_pixel(1, 1, OFF);
    let got = checked_with(&nine(), |l, files| {
        let (t, png) = tile(
            SkinWindow::Chat,
            "plaque",
            &plaque,
            (TileAxis::Centre, TileAxis::Centre),
            true,
        )
        .unwrap()
        .unwrap();
        files.insert(t.image.clone(), png);
        l.tiles.push(t);
    });
    assert!(
        matches!(got, Err(SkinRule::ShapeContentTransparent { .. })),
        "{got:?}"
    );
    // The same overhang, opaque, is allowed over the content.
    let got = checked_with(&nine(), |l, files| {
        let (t, png) = tile(
            SkinWindow::Chat,
            "plaque",
            &solid(4, 4),
            (TileAxis::Centre, TileAxis::Centre),
            true,
        )
        .unwrap()
        .unwrap();
        files.insert(t.image.clone(), png);
        l.tiles.push(t);
    });
    assert_eq!(got, Ok(()));
}

#[test]
fn a_tile_over_the_content_area_is_refused() {
    // A content rectangle that starts above the bottom of the top row:
    // the top row's transparent pixels would cut into the content.
    let got = checked(&nine(), |l| l.content.top = 1);
    assert!(
        matches!(got, Err(SkinRule::ShapeIntrudes { .. })),
        "{got:?}"
    );
}

#[test]
fn a_tile_that_is_not_its_images_size_is_refused() {
    let got = checked(&nine(), |l| l.tiles[0].width += 1);
    assert!(matches!(got, Err(SkinRule::BadValue { .. })), "{got:?}");
}

#[test]
fn a_side_that_varies_every_row_hits_the_cap() {
    let mut p = nine();
    let side = RgbaImage::from_fn(4, 2, |x, y| if y == 1 && x == 0 { OFF } else { ON });
    p[3] = side;
    let got = checked(&p, |_| {});
    assert!(
        matches!(got, Err(SkinRule::ShapeTooManyRects { max: 4096, .. })),
        "{got:?}"
    );
}

/// The built-in's main window at `size` (the reference screenshot's main
/// window, `scratchpad/skinref/<skin>/buddy-list-1.png`): its rectangle
/// count and its first rows (the top corners' cut).
fn golden(id: &str, size: (u32, u32)) -> (usize, Vec<Rect>) {
    let (_, b) = builtin::find(id).unwrap();
    let package = package::read(b.bytes).unwrap();
    let m = manifest::parse(&package).unwrap();
    let files = images::validate(&package, &m).unwrap();
    let layout = m
        .windows
        .get(&SkinWindow::Main)
        .expect("a main window layout");
    let rects = silhouette(layout, &masks(layout, &files), size.0, size.1, SCALE_ONE).unwrap();
    (rects.len(), rects.into_iter().take(3).collect())
}

#[test]
fn builtin_silhouettes_match_their_goldens() {
    // Each at its reference's main window size; Onyx's MenuBar row
    // decoration fills y 110-127, Alloy's MAINBG the side band, and
    // Combat keeps the 7 px holes its reference shows (x 11-17, x 323-329).
    let got: Vec<_> = [
        ("builtin:xfire-classic", (605, 412)),
        ("builtin:spacecowboy", (365, 549)),
        ("builtin:symbiosis", (399, 555)),
        ("builtin:total-chaos", (471, 553)),
        ("builtin:stealthx", (380, 555)),
        ("builtin:xslimfire", (330, 492)),
        ("builtin:simple-x", (340, 430)),
        ("builtin:xglow", (350, 378)),
        ("builtin:onyx", (380, 437)),
        ("builtin:alloy", (330, 500)),
        ("builtin:combat", (340, 489)),
    ]
    .into_iter()
    .map(|(id, size)| (id, golden(id, size)))
    .collect();
    let want: Vec<(&str, (usize, Vec<Rect>))> = vec![
        (
            "builtin:xfire-classic",
            (
                113,
                vec![r(69, 0, 467, 1), r(62, 1, 481, 1), r(56, 2, 493, 1)],
            ),
        ),
        (
            "builtin:spacecowboy",
            (314, vec![r(41, 6, 3, 1), r(40, 7, 5, 10), r(16, 17, 19, 1)]),
        ),
        (
            "builtin:symbiosis",
            (39, vec![r(7, 0, 372, 1), r(5, 1, 376, 1), r(4, 2, 378, 1)]),
        ),
        (
            "builtin:total-chaos",
            (
                87,
                vec![r(199, 0, 72, 2), r(168, 2, 134, 1), r(167, 3, 136, 1)],
            ),
        ),
        (
            "builtin:stealthx",
            (11, vec![r(6, 0, 368, 1), r(4, 1, 372, 1), r(3, 2, 374, 1)]),
        ),
        (
            "builtin:xslimfire",
            (11, vec![r(6, 0, 318, 1), r(4, 1, 322, 1), r(3, 2, 324, 1)]),
        ),
        ("builtin:simple-x", (1, vec![r(0, 0, 340, 430)])),
        (
            "builtin:xglow",
            (13, vec![r(5, 0, 340, 1), r(4, 1, 342, 1), r(3, 2, 344, 1)]),
        ),
        (
            "builtin:onyx",
            (7, vec![r(4, 0, 372, 1), r(2, 1, 376, 1), r(1, 2, 378, 2)]),
        ),
        (
            "builtin:alloy",
            (21, vec![r(10, 0, 311, 1), r(9, 1, 313, 1), r(8, 2, 315, 1)]),
        ),
        (
            "builtin:combat",
            (46, vec![r(8, 0, 324, 1), r(6, 1, 329, 1), r(4, 2, 332, 1)]),
        ),
    ];
    assert_eq!(got, want);
}

#[test]
fn a_decoration_adds_its_art_outside_the_content_and_never_cuts_it() {
    // Space Cowboy's AGEAR (bottom left, Z=4) reaching into the content
    // with transparent pixels, its opaque column in a 2 px left outset
    // (Symbiosis's IndentRight strip, mirrored).
    let gear = RgbaImage::from_fn(9, 9, |x, _| if x < 2 { ON } else { OFF });
    let add = |l: &mut WindowLayout, files: &mut BTreeMap<String, Vec<u8>>| {
        l.outsets.left = 2;
        let start = TileAxis::Start { offset: 0 };
        let end = TileAxis::End { offset: 0 };
        let (mut t, png) = tile(SkinWindow::Chat, "gear", &gear, (start, end), true)
            .unwrap()
            .unwrap();
        t.decoration = Some(4);
        files.insert(t.image.clone(), png);
        l.tiles.push(t);
    };
    assert_eq!(checked_with(&nine(), add), Ok(()));
    let (mut layout, mut files) = from_nine(SkinWindow::Chat, &nine()).unwrap();
    add(&mut layout, &mut files);
    let rects = silhouette(&layout, &masks(&layout, &files), 22, 20, SCALE_ONE).unwrap();
    let inside = |x: u32, y: u32| {
        rects
            .iter()
            .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    };
    // The gear's column in the outset, bottom 9 rows; nothing above it.
    assert!((11..20).all(|y| inside(0, y) && inside(1, y)));
    assert!(!inside(0, 10));
    // The content rectangle (frame 2..22, insets 4) stays whole.
    assert!((4..16).all(|y| (6..18).all(|x| inside(x, y))));
}
