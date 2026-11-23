//! The title backing over the titlebar strip (plan E7.7.7).

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, RegionFit};

use super::*;
use crate::serve::active_resolved;

/// A horizontal 3-slice with 10 px caps.
const STRIP: RegionFit = RegionFit::NineSlice(Insets {
    top: 0,
    right: 10,
    bottom: 0,
    left: 10,
});

const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A 40 × 56 strip: `band` on the rows the title covers at the chrome
/// height (source rows 13–43 at 2:1), `edge` above and below.
fn strip(band: [u8; 4], edge: [u8; 4]) -> RgbaImage {
    RgbaImage::from_fn(40, 56, |_, y| {
        Rgba(if (10..46).contains(&y) { band } else { edge })
    })
}

fn ground(image: &RgbaImage) -> TitleGround {
    let worst = strip_worst(image, STRIP, 28, None, WHITE).expect("a strip");
    TitleGround {
        worst_x100: scaled(worst, 100.0),
        required_x100: 450,
    }
}

#[test]
fn a_dark_strip_under_the_title_drops_its_backing() {
    // White text over near-black under the line box; the light rows at the
    // top and bottom edge are outside the title's line box.
    let g = ground(&strip([16, 16, 16, 255], [230, 230, 230, 255]));
    assert!(g.worst_x100 > 1500, "{g:?}");
    assert!(g.bare(), "{g:?}");
}

#[test]
fn a_light_strip_under_the_title_keeps_its_backing() {
    let g = ground(&strip([200, 200, 200, 255], [16, 16, 16, 255]));
    assert!(g.worst_x100 < 450, "{g:?}");
    assert!(!g.bare(), "{g:?}");
}

#[test]
fn a_translucent_strip_is_measured_over_a_white_desktop() {
    // Black at 50% over white is mid-grey: white text fails there.
    let g = ground(&strip([0, 0, 0, 128], [0, 0, 0, 128]));
    assert!(!g.bare(), "{g:?}");
}

#[test]
fn the_skin_layer_names_the_title_backing_only_when_bare() {
    for b in [
        "xslimfire",
        "xfire-classic",
        "symbiosis",
        "onyx",
        "stealthx",
        "spacecowboy",
        "total-chaos",
        "alloy",
        "simple-x",
        "xglow",
        "combat",
    ] {
        let (g, css) = builtin_title(b);
        assert_eq!(
            g.is_some_and(|g| g.bare()),
            css.contains("-title-backing: transparent")
        );
    }
}

/// Built-in `b`'s title ground and the title CSS its skin layer carries.
fn builtin_title(b: &str) -> (Option<TitleGround>, String) {
    let (_, builtin) =
        crate::builtin::find(&format!("{}{b}", crate::builtin::PREFIX)).expect("a built-in");
    let p = crate::package::read(builtin.bytes).expect("package");
    let m = crate::manifest::parse(&p).expect("manifest");
    let (_, resolved) = active_resolved(&p, &m).expect("tokens");
    let images: BTreeMap<String, Vec<u8>> =
        p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect();
    (
        title_ground(&m, &images, &resolved),
        title_css(&m, &images, &resolved),
    )
}

#[test]
fn xslimfire_measures_its_title_on_the_cropped_strip() {
    // The importer crops `MainTitleBar` to its 37 title rows
    // (`import/xfire_xml/title_rows.rs`); measured on the whole 106-row
    // art, the status bar's rows squashed under the title gave 2.08:1.
    // The title rows' own light art gives 1.27:1 under white text, below
    // SC 1.4.3's 4.5:1, so the label keeps its backing.
    let (g, css) = builtin_title("xslimfire");
    let g = g.expect("a titlebar strip");
    assert_eq!(g.worst_x100, 127, "{g:?}");
    assert!(!g.bare(), "{g:?}");
    assert_eq!(g.bare(), css.contains("-title-backing: transparent"));
}

/// Strips drawn wider and narrower than their source, caps that overlap
/// at a narrow width, and the built-ins' shapes (xSLIMFIRE's 266 × 37
/// strip at a 28 px chrome scales its caps down): `((iw, ih), (left,
/// right), chrome)`.
const SHAPES: [((u32, u32), (u32, u32), u32); 6] = [
    ((40, 56), (10, 10), 28),
    ((266, 37), (120, 120), 28),
    ((266, 37), (3, 200), 24),
    ((9, 30), (2, 3), 60),
    ((500, 20), (0, 0), 28),
    ((100, 10), (40, 1), 40),
];

/// The output widths the column tests draw each shape at.
fn outs() -> impl Iterator<Item = u32> {
    (1u32..700).chain([1024, 1999, 4096])
}

/// The [`strip_col`] evaluations `f` makes on this thread.
fn col_evals(f: impl FnOnce()) -> u64 {
    COL_EVALS.with(|n| n.set(0));
    f();
    COL_EVALS.with(std::cell::Cell::get)
}

/// The most [`strip_col`] evaluations one window width may cost a strip
/// `iw` px wide: a piece drawn at least as wide as its source evaluates
/// its two ends, a narrower one each of its output columns, which are no
/// more than its source columns; so the three pieces cost at most the
/// strip's width plus their six ends, whatever the window's width.
fn per_width_bound(iw: u32) -> u64 {
    u64::from(iw) + 6
}

#[test]
fn one_width_of_title_columns_costs_the_strips_width_not_the_windows() {
    // The regression this guards (plan E7.7.7, `serve.rs` SERVE_BUDGET):
    // one evaluation per output column, over every window width, took
    // Xfire Classic 2.8 s and xSLIMFIRE 3.6 s per stylesheet.
    for ((iw, ih), (left, right), chrome) in SHAPES {
        for out in outs() {
            let span = out.saturating_sub(2 * PAD_X) / 2;
            let mut hit = vec![false; iw as usize];
            let xs = PAD_X..PAD_X + span;
            let evals = col_evals(|| {
                mark_title_cols(&mut hit, xs, out, (iw, ih), (left, right), chrome);
            });
            assert!(
                evals <= per_width_bound(iw),
                "{iw}x{ih} caps {left}/{right} chrome {chrome} out {out}: {evals} evaluations"
            );
        }
    }
}

#[test]
fn the_title_measure_over_every_window_width_is_bounded_by_the_strip() {
    // `strip_worst` marks the title's columns at every width of every
    // window (`WINDOWS`), so its whole column work is bounded by the
    // widths times the strip's width, never by the widths' own columns.
    let widths: u64 = WINDOWS
        .iter()
        .map(|w| u64::from(w.max.0.max(w.min.0) - w.min.0 + 1))
        .sum();
    for ((iw, ih), (left, right), chrome) in SHAPES {
        let insets = |v: u32| u16::try_from(v).expect("a cap");
        let fit = RegionFit::NineSlice(Insets {
            top: 0,
            right: insets(right),
            bottom: 0,
            left: insets(left),
        });
        let image = RgbaImage::from_fn(iw, ih, |_, _| Rgba([16, 16, 16, 255]));
        let evals = col_evals(|| {
            strip_worst(&image, fit, chrome, None, WHITE).expect("a strip");
        });
        assert!(
            evals <= widths * per_width_bound(iw),
            "{iw}x{ih} caps {left}/{right} chrome {chrome}: {evals} evaluations over {widths} widths"
        );
    }
}

#[test]
fn the_piecewise_columns_are_the_per_column_walk() {
    for ((iw, ih), (left, right), chrome) in SHAPES {
        for out in outs() {
            let span = out.saturating_sub(2 * PAD_X) / 2;
            let xs = PAD_X..PAD_X + span;
            let walked: BTreeSet<u32> = xs
                .clone()
                .map(|x| strip_col(x, out, (iw, ih), (left, right), chrome))
                .collect();
            let mut hit = vec![false; iw as usize];
            mark_title_cols(&mut hit, xs, out, (iw, ih), (left, right), chrome);
            let marked: BTreeSet<u32> = (0..iw).filter(|&c| hit[c as usize]).collect();
            assert_eq!(
                marked, walked,
                "{iw}x{ih} caps {left}/{right} chrome {chrome} out {out}"
            );
        }
    }
}
