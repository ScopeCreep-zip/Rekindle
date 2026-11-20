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
    let got: Vec<_> = [
        ("builtin:xfire-classic", (605, 412)),
        ("builtin:spacecowboy", (365, 549)),
        ("builtin:symbiosis", (386, 555)),
        ("builtin:total-chaos", (471, 553)),
    ]
    .into_iter()
    .map(|(id, size)| (id, golden(id, size)))
    .collect();
    let want: Vec<(&str, (usize, Vec<Rect>))> = vec![
        (
            "builtin:xfire-classic",
            (
                112,
                vec![r(69, 0, 467, 1), r(62, 1, 481, 1), r(56, 2, 493, 1)],
            ),
        ),
        // Open (SK0 S1): the Named main-* pieces are read now, but only a
        // 1 px line of them is opaque, so the silhouette is the content
        // rectangle; the reference shows cut corners and overhang art.
        ("builtin:spacecowboy", (1, vec![r(15, 33, 331, 464)])),
        // Open (SK0 S1): the Sides shape does not match yet; still a
        // rectangle against the reference's rounded frame.
        ("builtin:symbiosis", (1, vec![r(0, 0, 386, 555)])),
        // The T.C plaque (72 px opaque top, centred) over the title row.
        (
            "builtin:total-chaos",
            (
                87,
                vec![r(199, 0, 72, 2), r(168, 2, 134, 1), r(167, 3, 136, 1)],
            ),
        ),
    ];
    assert_eq!(got, want);
}
