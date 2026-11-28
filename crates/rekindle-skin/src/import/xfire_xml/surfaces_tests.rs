//! Plan E7.7.7 S1: an XML-era tree's buddy-list components land in the
//! surface slots (a synthetic tree in Symbiosis's shape).

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, RegionFit, RegionState};

use super::*;
use crate::{manifest as parse_manifest, package, validate};

pub(super) fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([shade, shade, shade, 255]));
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

/// `Components/MainWindow.xml` with a status bar of 3 + 2 + 5 px pieces,
/// a sticky tab button (normal, hover, down), and a component no slot
/// takes.
fn tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "Skin.xml".into(),
        br#"<XfireSkin><Include File="Themes.xml"/><Include File="Components/MainWindow.xml"/></XfireSkin>"#.to_vec(),
    );
    let mut bitmaps = String::new();
    for (name, w, h, shade) in [
        ("sb_l", 3, 19, 40),
        ("sb_m", 2, 19, 50),
        ("sb_r", 5, 19, 60),
        ("tab_n", 20, 16, 70),
        ("tab_h", 20, 16, 80),
        ("tab_d", 20, 16, 90),
        ("logo", 30, 10, 100),
    ] {
        let _ = write!(
            bitmaps,
            "<Bitmap Name=\"{name}\">Images/{name}.png</Bitmap>"
        );
        t.insert(format!("Images/{name}.png"), png(w, h, shade));
    }
    t.insert(
        "Themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors><Color Name=\"ListBGCG\">RGBA(22,22,22,0)</Color></Colors><Bitmaps>{bitmaps}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "Components/MainWindow.xml".into(),
        br#"<Components>
<Component Name="StatusBarContainer" Class="Tile"><Images State="Normal"><Image Type="TopLeft">sb_l</Image><Image Type="Top">sb_m</Image><Image Type="TopRight">sb_r</Image></Images></Component>
<Component Name="ButtonFriendsTab" Class="PushButton" PushType="Sticky" GroupId="8765">
<Images State="Normal"><Image Type="Middle">tab_n</Image></Images>
<Images State="Hover"><Image Type="Middle">tab_h</Image></Images>
<Images State="Down"><Image Type="Middle">tab_d</Image></Images></Component>
<Component Name="XfireLogo" Class="SkinLauncher"><Images State="Normal"><Image Type="Middle">logo</Image></Images><Images State="Hover"><Image Type="Middle">logo</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t
}

#[test]
fn the_buddy_list_components_land_in_their_surface_slots() {
    let t = tree();
    let a = import(&t, "fixture", None).unwrap();
    assert_eq!(a, import(&t, "fixture", None).unwrap(), "deterministic");
    let canonical = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(canonical.bytes, a.canonical.bytes);
    let pkg = package::read(&a.canonical.bytes).unwrap();
    let m = parse_manifest::parse(&pkg).unwrap();

    // The bar: three pieces side by side, the ends as insets, backed by
    // its declared ground.
    let bar = &m.regions["statusbar"];
    assert_eq!(
        bar.fit,
        RegionFit::NineSlice(Insets {
            top: 0,
            right: 5,
            bottom: 0,
            left: 3
        })
    );
    assert!(bar.states.is_empty());
    assert_eq!(bar.text_backing.as_deref(), Some("bg.status"));

    // The sticky tab: its Down image is also the chosen tab's.
    let tab = &m.regions["buddylist.tab"];
    assert_eq!(
        tab.fit,
        RegionFit::NineSlice(Insets {
            top: surfaces::BUTTON_INSET,
            right: surfaces::BUTTON_INSET,
            bottom: surfaces::BUTTON_INSET,
            left: surfaces::BUTTON_INSET
        })
    );
    let states: Vec<_> = tab.states.keys().copied().collect();
    assert_eq!(
        states,
        [RegionState::Hover, RegionState::Down, RegionState::Selected]
    );
    let image = |path: &str| pkg.get(path).unwrap().to_vec();
    assert_eq!(
        image(&tab.states[&RegionState::Selected]),
        image(&tab.states[&RegionState::Down])
    );
    assert_eq!(tab.text_backing.as_deref(), Some("bg.dark"));

    // The logo launcher has no surface: dropped and named.
    assert!(!m.regions.keys().any(|k| k.contains("logo")));
    assert!(a.notice.contains("dropped XfireLogo Hover"));
    assert!(a
        .notice
        .contains("buddylist.tab: the component ButtonFriendsTab"));
}
