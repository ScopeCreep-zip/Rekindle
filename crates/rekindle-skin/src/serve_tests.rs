//! [`crate::serve::stylesheet`]'s work per built-in, counted (plan E7.7.7,
//! FLK3): the operation counts gate, the wall clock is only reported
//! (`tests/serve_budget.rs`, `cargo xtask perf`). Each bound says why the
//! stage's work cannot grow with what it must not grow with; the pinned
//! table makes any change in work a diff to explain.

use crate::regions::{frames, slot_of, SLOTS, WINDOWS};
use crate::work::{measure, Work};
use crate::{builtin, manifest, package, serve, validate};

/// The window × side cases the region check measures per frame: each
/// window of [`WINDOWS`] at its minimum and its maximum size.
fn cases() -> u64 {
    2 * WINDOWS.len() as u64
}

/// The window widths the title measure walks: every width of every window.
fn widths() -> u64 {
    WINDOWS
        .iter()
        .map(|w| u64::from(w.max.0.max(w.min.0) - w.min.0 + 1))
        .sum()
}

/// The window heights, as [`widths`].
fn heights() -> u64 {
    WINDOWS
        .iter()
        .map(|w| u64::from(w.max.1.max(w.min.1) - w.min.1 + 1))
        .sum()
}

/// The most text tokens a slot measures.
fn max_texts() -> u64 {
    SLOTS.iter().map(|s| s.text.len() as u64).max().unwrap_or(0)
}

/// What a pack holds that bounds its work: its distinct image files, the
/// pixels of all their frames, and the title strip's per-width column
/// bound summed over the strips the title measure walks (per text token,
/// state and frame; `regions/title_tests.rs` `per_width_bound`).
struct Pack {
    images: u64,
    pixels: u64,
    strip_cols: u64,
}

fn pack_of(bytes: &[u8]) -> Pack {
    let p = package::read(bytes).expect("a package");
    let m = manifest::served(&p).expect("a manifest");
    let mut images = 0;
    let mut pixels = 0;
    for (name, b) in p.files() {
        let image = std::path::Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("gif"));
        if image {
            images += 1;
            for f in frames(name, b).expect("frames") {
                pixels += u64::from(f.width()) * u64::from(f.height());
            }
        }
    }
    let mut strip_cols = 0;
    if let (Some(region), Some(slot)) = (m.regions.get("titlebar"), slot_of("titlebar")) {
        for path in std::iter::once(&region.image).chain(region.states.values()) {
            for f in frames(path, p.get(path).expect("an image")).expect("frames") {
                strip_cols += slot.text.len() as u64 * (u64::from(f.width()) + 6);
            }
        }
    }
    Pack {
        images,
        pixels,
        strip_cols,
    }
}

/// Each built-in's measured work at FLK3 (`col_evals`, contrast visits,
/// contrast pixels, contrast calls, region cases, label spans, decodes,
/// decoded frames). The region check measures at validation, not here,
/// so its cases are 0; a pack whose title is not a 3-slice strip walks no
/// title columns. SK15: a pack whose Communities clone copies a worded
/// tab (Total Chaos, xslimfire, Alloy, XGlow) decodes that tab's art once
/// more, to measure the copied word's style (`skin_tree::label_style`).
/// DIS2: decodes = the stylesheet's own + each distinct image of the
/// (Normal, Disabled) piece pairs whose hashes differ, decoded once per
/// serve to measure whether the Disabled look is visibly the pack's
/// (`skin_tree::disabled`): Symbiosis 33 + 70, xfire-classic 24 + 22,
/// Onyx 10 + 28, Space Cowboy 6 + 28, StealthX 6 + 38, Total Chaos 11 + 30,
/// xslimfire 10 + 30; Combat, Alloy, Simple X and XGlow ship no Disabled
/// state (+ 0). DIS3: + each distinct package image the served refusal
/// copies decode that no pair decoded (the Normal pieces of a node that
/// reaches `skin_tree::dim::copy`, else its nearest painted ancestor's
/// pieces it crops), once per serve: Symbiosis 103 + 16, xfire-classic
/// 46 + 40, Combat 15 + 23, Onyx 38 + 11, Space Cowboy 34 + 16, StealthX
/// 44 + 37, Total Chaos 41 + 16 (28 decoded, 12 already a pair's), xslimfire
/// 40 + 11, Alloy 16 + 27, Simple X 12 + 21, XGlow 7 + 25; each a still
/// image (decoded frames = decodes). The copies' own pixels are made from
/// those frames, never decoded.
const PINNED: [(&str, [u64; 8]); 11] = [
    ("symbiosis", [39220, 5817, 19277, 26, 0, 245_125, 119, 119]),
    (
        "xfire-classic",
        [705_960, 40618, 79010, 24, 0, 215_710, 86, 86],
    ),
    ("combat", [0, 66146, 86310, 15, 0, 147_075, 38, 38]),
    ("onyx", [0, 660, 2496, 10, 0, 98050, 49, 49]),
    ("spacecowboy", [0, 3336, 6966, 6, 0, 58830, 50, 50]),
    ("stealthx", [0, 3522, 5040, 6, 0, 58830, 81, 81]),
    ("total-chaos", [0, 715_159, 1_117_134, 51, 0, 68635, 57, 57]),
    ("xslimfire", [1_823_730, 10382, 31110, 9, 0, 68635, 51, 51]),
    ("alloy", [0, 526, 1734, 12, 0, 117_660, 43, 43]),
    ("simple-x", [0, 4000, 8514, 12, 0, 117_660, 33, 33]),
    ("xglow", [0, 32, 126, 3, 0, 29415, 32, 32]),
];

fn row(w: Work) -> [u64; 8] {
    [
        w.col_evals,
        w.contrast_visits,
        w.contrast_pixels,
        w.contrast_calls,
        w.region_cases,
        w.label_spans,
        w.decodes,
        w.decoded_frames,
    ]
}

#[test]
fn every_builtins_stylesheet_work_is_bounded_by_its_images_not_the_windows() {
    let mut seen = Vec::new();
    for b in &builtin::PACKS {
        let c = validate::run(b.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", b.id));
        let address = c.address();
        let pack = pack_of(&c.bytes);
        let w = measure(|| {
            serve::stylesheet(&c.bytes, &address).expect("a stylesheet");
        });
        let id = b.id;
        // The title's columns: per window width, at most each strip's
        // width plus its six piece ends, never the width's own columns.
        assert!(w.col_evals <= widths() * pack.strip_cols, "{id}: {w:?}");
        // No contrast call visits more pixels than its image holds.
        assert!(w.contrast_visits <= w.contrast_pixels, "{id}: {w:?}");
        // The contrast work: every image pixel at most once per window ×
        // side case and text token.
        assert!(
            w.contrast_visits <= pack.pixels * cases() * max_texts(),
            "{id}: {w:?}"
        );
        // The region check: the window × side cases of each decoded frame.
        assert!(w.region_cases <= cases() * w.decoded_frames, "{id}: {w:?}");
        // The label boxes: the windows' widths and heights per frame and
        // text token measured.
        assert!(
            w.label_spans <= (widths() + heights()) * w.decoded_frames * max_texts(),
            "{id}: {w:?}"
        );
        // Each image decoded at most once per stylesheet.
        assert!(w.decodes <= pack.images, "{id}: {w:?}");
        seen.push((id, row(w)));
    }
    assert_eq!(seen, PINNED.to_vec(), "the work changed: explain the diff");
}
