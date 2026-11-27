//! The composited silhouette (plan SK0: the window's shape follows what is
//! painted). The webview composites overlapping layers source-over, so two
//! layers each under [`ALPHA_OPAQUE`] can paint a pixel above it: the
//! shaper decides from the composited alpha, never from each layer alone.

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{SkinWindow, TileAxis, WindowLayout};

use super::*;
use crate::{builtin, images, manifest, package};

/// Whether pixel (`x`, `y`) lies in `rects`.
fn covers(rects: &[Rect], x: u32, y: u32) -> bool {
    rects
        .iter()
        .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

/// `rects` as one flag per pixel of a `w × h` window, row-major.
fn bitmap(rects: &[Rect], w: u32, h: u32) -> Vec<bool> {
    let mut out = vec![false; (w * h) as usize];
    for r in rects {
        for y in r.y..(r.y + r.height).min(h) {
            for x in r.x..(r.x + r.width).min(w) {
                out[(y * w + x) as usize] = true;
            }
        }
    }
    out
}

/// Each layer thresholded alone: the masks less their faint pixels.
fn alone(masks: &[Mask]) -> Vec<Mask> {
    masks
        .iter()
        .map(|m| Mask::from_parts(m.width(), m.height(), m.bits().to_vec(), Vec::new()).unwrap())
        .collect()
}

/// A built-in's served window layout: (pack, window, layout, masks, the
/// served minimum size).
type Builtin = (
    &'static str,
    SkinWindow,
    WindowLayout,
    Vec<Mask>,
    (u32, u32),
);

/// Each built-in's served window layouts.
fn builtins() -> Vec<Builtin> {
    let mut out = Vec::new();
    for b in &builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let m = manifest::served(&package).unwrap();
        let files = images::validate(&package, &m).unwrap();
        for (window, layout) in m.windows {
            let masks = of_layout("windows", &layout, &files).unwrap();
            let o = layout.outsets;
            let min = (
                u32::from(layout.min_width) + u32::from(o.left) + u32::from(o.right),
                u32::from(layout.min_height) + u32::from(o.top) + u32::from(o.bottom),
            );
            out.push((b.id, window, layout, masks, min));
        }
    }
    out
}

#[test]
fn a_mask_keeps_its_faint_pixels() {
    let img = RgbaImage::from_fn(3, 2, |x, y| {
        Rgba([
            0,
            0,
            0,
            [0, 60, 200][x as usize] / (u8::try_from(y).unwrap() + 1),
        ])
    });
    let m = Mask::of(&img);
    assert_eq!(m.faint(), &[(1, 0, 60), (1, 1, 30), (2, 1, 100)]);
    assert_eq!(m.faint_row(1), &[(1, 1, 30), (2, 1, 100)]);
    assert!(m.get(2, 0) && !m.get(2, 1) && !m.get(1, 0));
    let parts = |faint| Mask::from_parts(3, 2, m.bits().to_vec(), faint);
    assert_eq!(parts(m.faint().to_vec()), Some(m.clone()));
    assert_eq!(parts(vec![(2, 0, 60)]), None, "on an opaque bit");
    assert_eq!(parts(vec![(1, 1, 30), (1, 0, 60)]), None, "out of order");
    assert_eq!(parts(vec![(1, 0, ALPHA_OPAQUE)]), None, "not faint");
    assert_eq!(parts(vec![(3, 0, 60)]), None, "outside");
}

/// Two copies of the top-left corner, each 127/255 alpha: each alone stays
/// outside the window, together they composite to 1 − (128/255)² ≈ 191.
#[test]
fn two_faint_layers_together_are_inside() {
    let on = |w, h| RgbaImage::from_pixel(w, h, Rgba([10, 20, 30, 255]));
    let pieces = [
        on(4, 4),
        on(2, 4),
        on(4, 4),
        on(4, 2),
        RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 0])),
        on(4, 2),
        on(4, 4),
        on(2, 4),
        on(4, 4),
    ];
    let (mut layout, files) = from_nine(SkinWindow::Main, &pieces).unwrap();
    let corner = &layout.tiles[0];
    assert_eq!((corner.width, corner.height), (4, 4));
    assert_eq!(
        (corner.x, corner.y),
        (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 })
    );
    let mut masks = of_layout("windows.main", &layout, &files).unwrap();
    let faint = Mask::of(&RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 127])));
    masks[0] = faint.clone();
    let one = silhouette(&layout, &masks, 10, 10, SCALE_ONE).unwrap();
    layout.tiles.push(layout.tiles[0].clone());
    masks.push(faint);
    let two = silhouette(&layout, &masks, 10, 10, SCALE_ONE).unwrap();
    for (x, y) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        assert!(
            !covers(&one, x, y),
            "one faint layer at ({x}, {y}): {one:?}"
        );
        assert!(
            covers(&two, x, y),
            "two faint layers at ({x}, {y}): {two:?}"
        );
    }
    assert!(covers(&one, 4, 0) && covers(&two, 9, 9));
}

/// Total Chaos's main window bevel (real app, Linux, the AD1 head): two
/// mirrored 1-px diagonals painted at alpha 139–159 where two layers
/// overlap, each under the threshold alone; measured at x 16–21 and
/// 447–452, y 10–15 (the diagonals run on to x 11 and 457, y 20).
const TOTAL_CHAOS_BEVEL: [(u32, u32); 6] = [
    (21, 10),
    (18, 13),
    (16, 15),
    (447, 10),
    (450, 13),
    (452, 15),
];

#[test]
fn total_chaos_main_bevel_is_inside_at_the_served_minimum() {
    let (_, _, layout, masks, (w, h)) = builtins()
        .into_iter()
        .find(|b| b.0 == "total-chaos" && b.1 == SkinWindow::Main)
        .unwrap();
    let rects = silhouette(&layout, &masks, w, h, SCALE_ONE).unwrap();
    let before = silhouette(&layout, &alone(&masks), w, h, SCALE_ONE).unwrap();
    for (x, y) in TOTAL_CHAOS_BEVEL {
        assert!(covers(&rects, x, y), "({x}, {y}) at {w}x{h} is cut out");
        assert!(!covers(&before, x, y), "({x}, {y}) is opaque in one layer");
    }
}

/// Every built-in window whose silhouette compositing changes, at the
/// served minimum and 300 px over it: (pack, window, width, height, rects
/// and area with each layer alone, rects and area composited). Only Total
/// Chaos changes: its main and community windows share the frame whose top
/// bevel is two mirrored 11-px diagonals of two overlapping faint layers
/// (x 11–21 and width − 23 … width − 13, y 10–20), 22 pixels at either
/// size; the rectangles drop by one as those rows merge.
const COMPOSITE_CHANGES: [(&str, &str, u32, u32, usize, u64, usize, u64); 4] = [
    ("total-chaos", "main", 470, 380, 107, 166_212, 106, 166_234),
    ("total-chaos", "main", 770, 680, 107, 500_412, 106, 500_434),
    (
        "total-chaos",
        "community",
        470,
        380,
        93,
        140_041,
        92,
        140_063,
    ),
    (
        "total-chaos",
        "community",
        770,
        680,
        93,
        456_241,
        92,
        456_263,
    ),
];

#[test]
fn compositing_only_adds_edge_pixels_and_the_changes_are_these() {
    let mut changes = Vec::new();
    let (mut wrong, mut log) = (Vec::new(), Vec::new());
    for (pack, window, layout, masks, min) in builtins() {
        let name = format!("{window:?}").to_ascii_lowercase();
        for (w, h) in [min, (min.0 + 300, min.1 + 300)] {
            let new = silhouette(&layout, &masks, w, h, SCALE_ONE).unwrap();
            let old = silhouette(&layout, &alone(&masks), w, h, SCALE_ONE).unwrap();
            if new == old {
                continue;
            }
            let area = |r: &[Rect]| {
                r.iter()
                    .map(|q| u64::from(q.width) * u64::from(q.height))
                    .sum::<u64>()
            };
            changes.push((
                pack,
                name.clone(),
                w,
                h,
                old.len(),
                area(&old),
                new.len(),
                area(&new),
            ));
            let (was, now) = (bitmap(&old, w, h), bitmap(&new, w, h));
            let mut added = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let at = (y * w + x) as usize;
                    if was[at] && !now[at] {
                        wrong.push(format!("{pack} {name} {w}x{h}: ({x}, {y}) dropped"));
                    }
                    if !was[at] && now[at] {
                        added.push((x, y));
                        let edge = (y.saturating_sub(1)..=(y + 1).min(h - 1)).any(|ny| {
                            (x.saturating_sub(1)..=(x + 1).min(w - 1))
                                .any(|nx| was[(ny * w + nx) as usize])
                        });
                        if !edge {
                            wrong.push(format!(
                                "{pack} {name} {w}x{h}: ({x}, {y}) is not on an edge"
                            ));
                        }
                    }
                }
            }
            log.push(format!(
                "{pack} {name} {w}x{h}: {} added: {added:?}",
                added.len()
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let want: Vec<_> = COMPOSITE_CHANGES
        .iter()
        .map(|&(p, n, w, h, a, b, c, d)| (p, n.to_owned(), w, h, a, b, c, d))
        .collect();
    assert_eq!(
        changes,
        want,
        "every change compositing makes:\n{}",
        log.join("\n")
    );
}
