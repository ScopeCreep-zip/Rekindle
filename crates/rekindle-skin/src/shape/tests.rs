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
    // StealthX is 400 wide with its menu drawer (rightmost opaque column
    // 399 in the reference's top and bottom rows, where no Infoview is
    // attached); whole-window opacity XOR 196/222,000 px, all at the
    // drawer's and the body's bottom-left corner cuts (y 527-555), SK2-P3.
    // SK2-P1d: the drawer is the `MainMenuPopup` rail (`popups.xml:2-89`),
    // laid in its own box, so the main tree's silhouette is the body alone
    // (x 112-399, 11 rects): the drawer's band (x 0-111, y 29-526) left it.
    // xSLIMFIRE and Alloy are 348 wide: their top-right/bottom-right
    // corner pieces' rightmost opaque columns sit at 347 in the reference's
    // top and bottom rows (no Infoview attached there), SK2-P4. Opacity
    // XOR: xSLIMFIRE 4/171,216 px, Alloy 44/174,000 px, every one a
    // reference outline pixel at alpha 106-126 (under the 128 cut) save
    // Alloy's two right corner tips, (339, 0) and (338, 499), one opaque
    // pixel more in the reference's art than in its pieces.
    // Classic is 501x549 (its original at 0.75, 376x412), SK2-P4; with its
    // frame drawn only by the tree's rows (SK2-P5 fix 1) its opacity XOR
    // is 2,656/154,912 px: x 366-375 (window x 488-500) y 50-188 and
    // 222-349, 2,554 px, the 13 px strip right of `MainRightOuterEdge`
    // where the original draws its Infoview tab (spec: the strip the root
    // `IndentRight="13+14"` leaves), and 102 px in 26 regions under 30 px
    // along the outline, the original's 0.75 resampling.
    // SK2-P8 (opacity XOR against the reference, the cut at alpha 128):
    // Total Chaos is 470x555 (decision 29: the rows' `OutdentLeft` column
    // is cut, not added; the reference is opaque down to row 554), 541 px,
    // every one at reference alpha 1-254 (494 under the 128 cut that we
    // paint, 47 over it that we leave clear): its soft outline and the
    // bottom band's shadow, none at alpha 0 or 255. SK2-P9 (decision 32):
    // the `BottomCenter` tab centred in the uncut 471 px `BottomBar` sits
    // at x 149, so the 11 alpha-0 pixels of its left diagonal (x 170-180,
    // y 544-554) are gone and its right diagonal's 11 at alpha 111-116
    // (x 290-299) join the soft outline. Space Cowboy is the
    // 380x555 window (`DefaultHeight`/`MinHeight` 555) whose screenshot
    // lacks its top 6 rows (reference row y = window row y + 6): 7,930 px
    // in x 360-379, the separate `InfoPopup` window (`popups.xml:2`), and
    // 534 at reference alpha 1-127. Xglow is 348x379: 19 px, all at
    // reference alpha 1-127 in the four corner cuts.
    let got: Vec<_> = [
        ("builtin:xfire-classic", (501, 549)),
        ("builtin:spacecowboy", (380, 555)),
        ("builtin:symbiosis", (399, 555)),
        ("builtin:total-chaos", (470, 555)),
        ("builtin:stealthx", (400, 555)),
        ("builtin:xslimfire", (348, 492)),
        ("builtin:simple-x", (340, 430)),
        ("builtin:xglow", (348, 379)),
        ("builtin:onyx", (380, 437)),
        ("builtin:alloy", (348, 500)),
        ("builtin:combat", (340, 489)),
    ]
    .into_iter()
    .map(|(id, size)| (id, golden(id, size)))
    .collect();
    let want: Vec<(&str, (usize, Vec<Rect>))> = vec![
        (
            "builtin:xfire-classic",
            (
                126,
                vec![r(83, 0, 322, 1), r(76, 1, 336, 1), r(70, 2, 348, 1)],
            ),
        ),
        (
            "builtin:spacecowboy",
            (308, vec![r(41, 6, 3, 1), r(40, 7, 5, 10), r(16, 17, 19, 1)]),
        ),
        (
            "builtin:symbiosis",
            (39, vec![r(7, 0, 372, 1), r(5, 1, 376, 1), r(4, 2, 378, 1)]),
        ),
        (
            "builtin:total-chaos",
            (
                // 106 since SK0 composites faint layers: the top bevel's two
                // diagonals join (`composite_tests`), merging one row group.
                106,
                vec![r(198, 0, 72, 2), r(167, 2, 134, 1), r(166, 3, 136, 1)],
            ),
        ),
        (
            "builtin:stealthx",
            (
                11,
                vec![r(118, 0, 276, 1), r(116, 1, 280, 1), r(115, 2, 282, 1)],
            ),
        ),
        (
            "builtin:xslimfire",
            (11, vec![r(6, 0, 336, 1), r(4, 1, 340, 1), r(3, 2, 342, 1)]),
        ),
        ("builtin:simple-x", (1, vec![r(0, 0, 340, 430)])),
        (
            "builtin:xglow",
            (13, vec![r(5, 0, 338, 1), r(4, 1, 340, 1), r(3, 2, 342, 1)]),
        ),
        (
            "builtin:onyx",
            (7, vec![r(4, 0, 372, 1), r(2, 1, 376, 1), r(1, 2, 378, 2)]),
        ),
        (
            "builtin:alloy",
            (21, vec![r(10, 0, 329, 1), r(9, 1, 331, 1), r(8, 2, 333, 1)]),
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

/// `nine()` with a centre fill tile of `art` at (`x`, `y`) and the content's
/// left inset `left` (plan SK0-L1).
fn with_fill(
    art: &RgbaImage,
    x: TileAxis,
    y: TileAxis,
    left: u16,
) -> impl Fn(&mut WindowLayout, &mut BTreeMap<String, Vec<u8>>) + '_ {
    move |l, files| {
        let (mut t, png) = tile(SkinWindow::Chat, "band", art, (x, y), false)
            .unwrap()
            .unwrap();
        t.fill = true;
        files.insert(t.image.clone(), png);
        l.tiles.push(t);
        l.content.left = left;
        l.min_width = 16;
    }
}

/// Rule 2's fill band (plan SK0-L1, lead decision for Space Cowboy): a
/// wholly opaque centre fill beside the content, or above it with its edge
/// on the content's top line, passes; one over the content, one with a
/// transparent pixel, or one above the content clear of it is refused.
#[test]
fn a_centre_fill_is_allowed_only_opaque_in_the_band_beside_the_content() {
    let art = solid(3, 2);
    let (at, down) = (
        TileAxis::Start { offset: 4 },
        TileAxis::Repeat { start: 4, end: 4 },
    );
    assert_eq!(checked_with(&nine(), with_fill(&art, at, down, 7)), Ok(()));
    let refused = Err(SkinRule::BadValue {
        field: "windows.chat.tiles.8".to_owned(),
        expected:
            "a centre fill wholly opaque in the band between the frame pieces and the content"
                .to_owned(),
    });
    assert_eq!(checked_with(&nine(), with_fill(&art, at, down, 4)), refused);
    let mut holed = art.clone();
    holed.put_pixel(2, 1, OFF);
    assert_eq!(
        checked_with(&nine(), with_fill(&holed, at, down, 7)),
        refused
    );
    let top = TileAxis::Start { offset: 0 };
    assert_eq!(checked_with(&nine(), with_fill(&art, at, top, 7)), refused);
    // Under the 4 px top row, 2 px tall, the content's top moved to its
    // lower edge (Space Cowboy's chat window).
    let row = with_fill(&art, at, TileAxis::Start { offset: 4 }, 4);
    let on_the_top_line = move |l: &mut WindowLayout, files: &mut BTreeMap<String, Vec<u8>>| {
        row(l, files);
        l.content.top = 6;
        l.min_height = 12;
    };
    assert_eq!(checked_with(&nine(), on_the_top_line), Ok(()));
}

/// Space Cowboy's content is its views' rectangle (27/30 px in,
/// `mainwindow.xml` `TabbedControl`), its 16/20 px sides widened by the
/// pack's centre fill (plan SK0-L1, lead decision).
#[test]
fn space_cowboys_content_is_its_views_and_the_band_is_its_centre_fill() {
    let (_, b) = builtin::find("builtin:spacecowboy").unwrap();
    let m = manifest::parse(&package::read(b.bytes).unwrap()).unwrap();
    let main = &m.windows[&SkinWindow::Main];
    assert_eq!((main.content.left, main.content.right), (27, 30));
    let fills: Vec<_> = main
        .tiles
        .iter()
        .filter(|t| t.fill)
        .map(|t| (t.width, t.x))
        .collect();
    assert_eq!(
        fills,
        vec![
            (11, TileAxis::Start { offset: 16 }),
            (10, TileAxis::End { offset: 20 })
        ]
    );
}

#[test]
fn total_chaos_lays_its_content_region() {
    // SK2: the content is the views node's box. mainwindow.xml:22-31:
    // `Content` OutdentLeft="1" from `MenuBar.BOTTOM` (52 + 60 + 69 = 181)
    // to IndentBottom="61"; in it `TabbedControl` X="50" IndentRight="50"
    // at `TabHeader.BOTTOM` (181 + 22 = 203). Lead decision 29: the row's
    // outdented column is cut, so the row starts at the window's edge and
    // `TabbedControl` keeps its place, x = -1 + 50 = 49; lead decision 27:
    // the `BottomBar` is clipped to the 61 px under `Content` at the
    // window's bottom edge, so bottom 61, the row's `IndentBottom`.
    // SKW3 ruling (a): the pack declares a `StatusView` (`components.xml:898-902`,
    // MinHeight 20) but lays none, so the import lays it before the views
    // tile at `TabHeader.BOTTOM` and the views start under it: 203 + 20 = 223.
    let (_, b) = builtin::find("builtin:total-chaos").unwrap();
    let m = manifest::parse(&package::read(b.bytes).unwrap()).unwrap();
    let main = &m.windows[&SkinWindow::Main];
    let c = main.content;
    assert_eq!((c.top, c.right, c.bottom, c.left), (223, 50, 61, 49));
}

/// Lead decision 3 (plan SK2, 2026-11-20): under the pack's component tree
/// the window's art may lie under any component, the views included (the
/// views node draws opaque); without a tree SK0's rule 2 still refuses it.
#[test]
fn a_tree_laid_window_may_draw_its_art_under_the_views() {
    let mut plaque = solid(4, 4);
    plaque.put_pixel(1, 1, OFF);
    let lay = |l: &mut WindowLayout, files: &mut BTreeMap<String, Vec<u8>>| {
        let (t, png) = tile(
            SkinWindow::Chat,
            "plaque",
            &plaque,
            (TileAxis::Centre, TileAxis::Centre),
            false,
        )
        .unwrap()
        .unwrap();
        files.insert(t.image.clone(), png);
        l.tiles.push(t);
    };
    let got = checked_with(&nine(), lay);
    assert!(
        matches!(got, Err(SkinRule::ShapeIntrudes { .. })),
        "{got:?}"
    );
    let got = checked_with(&nine(), |l, files| {
        lay(l, files);
        l.tree = Some(rekindle_types::dto::skin_tree::SkinTree {
            outsets: rekindle_types::dto::skin::Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
            root: Vec::new(),
        });
    });
    assert_eq!(got, Ok(()));
}
