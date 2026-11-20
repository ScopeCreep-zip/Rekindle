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
        let (_, builtin) =
            crate::builtin::find(&format!("{}{b}", crate::builtin::PREFIX)).expect("a built-in");
        let p = crate::package::read(builtin.bytes).expect("package");
        let m = crate::manifest::parse(&p).expect("manifest");
        let (_, resolved) = active_resolved(&p, &m).expect("tokens");
        let images: BTreeMap<String, Vec<u8>> =
            p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect();
        let g = title_ground(&m, &images, &resolved);
        let css = title_css(&m, &images, &resolved);
        assert_eq!(
            g.is_some_and(|g| g.bare()),
            css.contains("-title-backing: transparent")
        );
    }
}
