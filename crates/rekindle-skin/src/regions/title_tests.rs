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

#[test]
fn the_piecewise_columns_are_the_per_column_walk() {
    // Strips drawn wider and narrower than their source, caps that
    // overlap at a narrow width, and the built-ins' shapes (xSLIMFIRE's
    // 266 × 37 strip at a 28 px chrome scales its caps down).
    let shapes = [
        ((40, 56), (10, 10), 28),
        ((266, 37), (120, 120), 28),
        ((266, 37), (3, 200), 24),
        ((9, 30), (2, 3), 60),
        ((500, 20), (0, 0), 28),
        ((100, 10), (40, 1), 40),
    ];
    for ((iw, ih), (left, right), chrome) in shapes {
        for out in (1u32..700).chain([1024, 1999, 4096]) {
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
