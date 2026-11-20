//! Silhouettes and the helper's refusals (plan SK0 S1).

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{SkinManifest, SkinRule, SkinWindow, WindowLayout};

use super::*;
use crate::{builtin, manifest, package};

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

#[test]
fn corners_cut_and_middles_repeat() {
    let (layout, files) = from_nine(SkinWindow::Main, &nine()).unwrap();
    assert_eq!(layout.tiles.len(), 8, "the centre is the content ground");
    assert_eq!(files.len(), 8);
    let want = vec![
        r(2, 0, 6, 1),
        r(1, 1, 8, 1),
        r(0, 2, 10, 6),
        r(1, 8, 8, 1),
        r(2, 9, 6, 1),
    ];
    assert_eq!(silhouette(&layout, 10, 10, SCALE_ONE).unwrap(), want);
    let doubled: Vec<Rect> = want
        .iter()
        .map(|q| r(q.x * 2, q.y * 2, q.width * 2, q.height * 2))
        .collect();
    assert_eq!(silhouette(&layout, 10, 10, 2 * SCALE_ONE).unwrap(), doubled);
    // Determinism: the same layout gives the same rectangles.
    assert_eq!(
        silhouette(&layout, 37, 23, 1250).unwrap(),
        silhouette(&layout, 37, 23, 1250).unwrap()
    );
}

#[test]
fn mask_round_trips_and_refuses_bad_text() {
    let img = corner(9, 4, (false, false));
    let m = Mask::of(&img);
    assert_eq!(Mask::from_hex(&m.to_hex(), 9, 9), Some(m.clone()));
    assert_eq!(Mask::from_hex(&m.to_hex().to_uppercase(), 9, 9), None);
    assert_eq!(Mask::from_hex("00", 9, 9), None);
}

fn base() -> SkinManifest {
    let (_, b) = builtin::find(builtin::DEFAULT).unwrap();
    manifest::parse(&package::read(b.bytes).unwrap()).unwrap()
}

fn checked(pieces: &[RgbaImage; 9], edit: impl Fn(&mut WindowLayout)) -> Result<(), SkinRule> {
    let (mut layout, files) = from_nine(SkinWindow::Chat, pieces).unwrap();
    edit(&mut layout);
    let mut m = base();
    m.windows = BTreeMap::from([(SkinWindow::Chat, layout)]);
    check(&m, &files).map_err(|e| e.rule)
}

#[test]
fn a_well_formed_frame_passes() {
    assert_eq!(checked(&nine(), |_| {}), Ok(()));
}

#[test]
fn a_hole_inside_the_frame_is_refused() {
    let mut p = nine();
    p[0].put_pixel(2, 2, OFF);
    assert!(matches!(
        checked(&p, |_| {}),
        Err(SkinRule::ShapeHole { x: 2, y: 2, .. })
    ));
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
fn a_mask_that_is_not_the_image_is_refused() {
    let got = checked(&nine(), |l| l.tiles[0].mask = "ff".repeat(4));
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

/// The built-in's main window at the reference screenshot's size
/// (`scratchpad/skinref/<skin>/buddy-list-1.png`): its rectangle count
/// and the top-left corner's first rows.
fn golden(id: &str, size: (u32, u32)) -> (usize, Vec<Rect>) {
    let (_, b) = builtin::find(id).unwrap();
    let m = manifest::parse(&package::read(b.bytes).unwrap()).unwrap();
    let layout = m
        .windows
        .get(&SkinWindow::Main)
        .expect("a main window layout");
    let rects = silhouette(layout, size.0, size.1, SCALE_ONE).unwrap();
    let n = rects.len();
    eprintln!(
        "{id}: {n} rects, {:?}",
        rects.iter().take(6).collect::<Vec<_>>()
    );
    (n, rects.into_iter().take(3).collect())
}

#[test]
fn builtin_silhouettes_match_their_goldens() {
    let got: Vec<_> = [
        ("builtin:xfire-classic", (605, 412)),
        ("builtin:spacecowboy", (598, 549)),
        ("builtin:symbiosis", (583, 555)),
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
        (
            // Space Cowboy and Symbiosis: their main pieces decode fully
            // opaque here, so the outline is the rectangle (an open finding:
            // the references show cut corners).
            "builtin:spacecowboy",
            (1, vec![r(0, 0, 598, 549)]),
        ),
        ("builtin:symbiosis", (1, vec![r(0, 0, 583, 555)])),
    ];
    assert_eq!(got, want);
}

#[test]
fn total_chaos_has_no_main_layout_yet() {
    // Its `MainTitleBar` (left/repeat/right) and centred T.C plaque match
    // none of the nine-piece shapes; the overhang import is SK0 S1's rest.
    let (_, b) = builtin::find("builtin:total-chaos").unwrap();
    let m = manifest::parse(&package::read(b.bytes).unwrap()).unwrap();
    assert!(!m.windows.contains_key(&SkinWindow::Main));
}
