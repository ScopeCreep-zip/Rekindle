//! The importer on a small in-memory Xfire tree.

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::validate;

const MAGENTA: Rgba<u8> = Rgba([0xff, 0, 0xff, 0xff]);

fn png(w: u32, h: u32, px: Rgba<u8>) -> Vec<u8> {
    let mut img = RgbaImage::from_pixel(w, h, px);
    img.put_pixel(0, 0, MAGENTA);
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

/// Carbon Fiber's shape: the frame split over components, the top row
/// carrying a `SideCG` shift.
fn tree(colours: &str) -> Tree {
    let mut t = Tree::new();
    t.insert(
        "Skin.xml".into(),
        br#"<?xml version="1.0"?><!DOCTYPE XfireSkin SYSTEM "x.dtd"><XfireSkin><Include File="Themes.xml"/><Include File="Components/ChatWindow.xml"/></XfireSkin>"#.to_vec(),
    );
    let mut bitmaps = String::new();
    for (i, n) in bitmap::NINE.iter().enumerate() {
        let _ = write!(bitmaps, "<Bitmap Name=\"p{i}\">Images/{n}.png</Bitmap>");
        let side = if *n == "Middle" { 8 } else { 3 };
        t.insert(
            format!("Images/{n}.png"),
            png(side, side, Rgba([40, 100, 160, 255])),
        );
    }
    t.insert(
        "Themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors>{colours}</Colors><Bitmaps>{bitmaps}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "Components/ChatWindow.xml".into(),
        br#"<Components>
<Component Name="Top" Class="Tile"><Triggers><Trigger Type="OnRightClick">ContextMenuSys</Trigger></Triggers>
<Images State="Normal" Color="SideCG"><Image Type="TopLeft">p0</Image><Image Type="Top">p1</Image><Image Type="TopRight">p2</Image></Images>
<Images State="Hover"><Image Type="Left">p0</Image></Images></Component>
<Component Name="Mid" Class="Tile"><Images State="Normal"><Image Type="Left">p3</Image><Image Type="Middle">p4</Image><Image Type="Right">p5</Image></Images></Component>
<Component Name="Bottom" Class="Tile"><Images State="Normal"><Image Type="BottomLeft">p6</Image><Image Type="Bottom">p7</Image><Image Type="BottomRight">p8</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t.insert(
        "Info.xml".into(),
        b"<SkinDescription><Name>Fixture</Name></SkinDescription>".to_vec(),
    );
    t
}

const PASSING: &str = r#"<Color Name="ListBGCG">RGBA(22,22,22,0)</Color>
<Color Name="ListFriendTextCG">RGBA(183,184,184,0)</Color>
<Color Name="XIGCaretCG">RGBA(255,255,255,0)</Color>
<Color Name="SideCG">RGBA(30,-30,0,0)</Color>"#;

#[test]
fn imports_validates_and_is_byte_identical() {
    let t = tree(PASSING);
    let a = import(&t, "fixture").unwrap();
    let b = import(&t, "fixture").unwrap();
    assert_eq!(a, b);
    assert!(a.changes.is_empty());
    let again = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(again.bytes, a.canonical.bytes);
    assert!(a
        .notice
        .contains("XIGCaretCG RGBA(255,255,255,0) -> dropped"));
    assert!(a.notice.contains("triggers: 1"));
}

#[test]
fn non_zero_alpha_is_refused() {
    let t = tree(&PASSING.replace("RGBA(22,22,22,0)", "RGBA(22,22,22,1)"));
    assert!(matches!(
        import(&t, "fixture"),
        Err(ImportError::NonZeroAlpha { alpha: 1, .. })
    ));
}

#[test]
fn shift_and_colour_key_are_baked() {
    let r = read(&tree(PASSING)).unwrap();
    let (img, insets) = r.frame.unwrap();
    assert_eq!(
        (insets.top, insets.left, img.width(), img.height()),
        (3, 3, 14, 14)
    );
    // The top row's pieces carry SideCG (+30, -30, 0); the rest do not.
    assert_eq!(img.get_pixel(4, 1).0, [70, 70, 160, 255]);
    assert_eq!(img.get_pixel(4, 5).0, [40, 100, 160, 255]);
    // Each piece's top-left #FF00FF pixel is keyed out.
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(img.get_pixel(3, 3).0, [0, 0, 0, 0]);
}

#[test]
fn a_failing_pair_is_walked_and_recorded() {
    let t = tree(&format!(
        "{PASSING}<Color Name=\"ListOfflineTextCG\">RGBA(12,51,87,0)</Color>"
    ));
    let a = import(&t, "fixture").unwrap();
    let c = a.changes.first().unwrap();
    assert_eq!(
        (c.token.as_str(), c.original),
        ("status.offline-text", [12, 51, 87])
    );
    assert!(c.before_x100 < 450);
    assert!(a.notice.contains("status.offline-text: #0c3357 -> "));
    validate::run(&a.canonical.bytes).unwrap();
}

#[test]
fn xfire_own_skins_carry_the_owner_ruling() {
    let mut t = tree(PASSING);
    let third = import(&t, "fixture").unwrap();
    assert!(third.notice.contains("third-party work"));
    assert!(!third.notice.contains("2026-10-15"));
    t.insert(
        "Info.xml".into(),
        br#"<SkinDescription><Author Type="Contributor">Someone</Author><Author Type="Original">Xfire, Inc.</Author><Name>Fixture</Name></SkinDescription>"#.to_vec(),
    );
    let own = import(&t, "fixture").unwrap();
    assert!(own.notice.contains("Xfire, Inc.'s own work"));
    assert!(own.notice.contains("2026-10-15"));
    assert!(own.notice.contains("MIT LICENSE does not cover"));
    let manifest =
        crate::manifest::parse(&crate::package::read(&own.canonical.bytes).unwrap()).unwrap();
    assert!(matches!(
        manifest.license,
        SkinLicense::Text(ref s) if s.starts_with("Xfire, Inc.'s own skin art")
    ));
}

/// `tree` plus a `Common.xml` with a close button (normal, hover, down,
/// and a disabled image `disabled_w` wide), a vertical scrollbar thumb in
/// two states, an arrow, and a baked-label button that stays dropped.
fn tree_with_controls(disabled_w: u32) -> Tree {
    let mut t = tree(PASSING);
    t.insert(
        "Skin.xml".into(),
        br#"<XfireSkin><Include File="Themes.xml"/><Include File="Components/ChatWindow.xml"/><Include File="Components/Common.xml"/></XfireSkin>"#.to_vec(),
    );
    let mut themes = String::from_utf8(t["Themes.xml"].clone()).unwrap();
    let mut bitmaps = String::new();
    let mut add = |name: &str, w: u32, h: u32, v: u8| {
        let _ = write!(
            bitmaps,
            "<Bitmap Name=\"{name}\">Images/{name}.png</Bitmap>"
        );
        t.insert(
            format!("Images/{name}.png"),
            png(w, h, Rgba([v, v, v, 255])),
        );
    };
    add("close_n", 18, 16, 60);
    add("close_h", 18, 16, 90);
    add("close_d", 18, 16, 30);
    add("close_x", disabled_w, 16, 50);
    for (s, v) in [("n", 70), ("h", 100)] {
        add(&format!("vt_{s}"), 12, 4, v);
        add(&format!("vm_{s}"), 12, 6, v);
        add(&format!("vb_{s}"), 12, 4, v);
        add(&format!("up_{s}"), 12, 12, v);
    }
    add("send_h", 40, 20, 80);
    themes = themes.replace("</Bitmaps>", &format!("{bitmaps}</Bitmaps>"));
    t.insert("Themes.xml".into(), themes.into_bytes());
    let state = |s: &str, x: &str| {
        format!("<Images State=\"{s}\" Color=\"SideCG\"><Image Type=\"ScrollVertGripperTop\">vt_{x}</Image><Image Type=\"ScrollVertGripperMiddle\">vm_{x}</Image><Image Type=\"ScrollVertGripperBottom\">vb_{x}</Image><Image Type=\"ScrollVertGripperOverlay\">vm_{x}</Image><Image Type=\"ScrollUp\">up_{x}</Image></Images>")
    };
    let common = format!(
        "<Components><Component Name=\"ButtonClose\" Class=\"PushButton\"><Images State=\"Normal\"><Image Type=\"Middle\">close_n</Image></Images><Images State=\"Hover\"><Image Type=\"Middle\">close_h</Image></Images><Images State=\"Down\"><Image Type=\"Middle\">close_d</Image></Images><Images State=\"Disabled\"><Image Type=\"Middle\">close_x</Image></Images></Component><Component Name=\"XfireScrollbar\" Class=\"Scrollbar\">{}{}</Component><Component Name=\"ChatButtonSend\" Class=\"PushButton\"><Images State=\"Normal\"><Image Type=\"Middle\">send_h</Image></Images><Images State=\"Hover\"><Image Type=\"Middle\">send_h</Image></Images></Component></Components>",
        state("Normal", "n"),
        state("Hover", "h")
    );
    t.insert("Components/Common.xml".into(), common.into_bytes());
    t
}

#[test]
fn title_buttons_and_the_scrollbar_keep_their_state_images() {
    let a = import(&tree_with_controls(18), "fixture").unwrap();
    let c = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(c.bytes, a.canonical.bytes);
    let r = read(&tree_with_controls(18)).unwrap();
    let close = r
        .controls
        .iter()
        .find(|c| c.region == "titlebar.btn.close")
        .unwrap();
    assert_eq!(close.fit, RegionFit::Stretch);
    let states: Vec<_> = close.images.keys().copied().collect();
    use rekindle_types::dto::skin::RegionState as S;
    assert_eq!(states, [S::Normal, S::Hover, S::Down, S::Disabled]);
    // Hover is `close_h` (90) shifted by nothing: the button set has no Color.
    assert_eq!(close.images[&S::Hover].get_pixel(5, 5).0, [90, 90, 90, 255]);
    let thumb = r
        .controls
        .iter()
        .find(|c| c.region == "scrollbar.thumb")
        .unwrap();
    assert_eq!(
        thumb.fit,
        RegionFit::NineSlice(rekindle_types::dto::skin::Insets {
            top: 4,
            right: 0,
            bottom: 4,
            left: 0
        })
    );
    assert_eq!(thumb.images[&S::Normal].dimensions(), (12, 14));
    // SideCG shifts the gripper (+30 red, -30 green).
    assert_eq!(
        thumb.images[&S::Hover].get_pixel(5, 5).0,
        [130, 70, 100, 255]
    );
    assert!(r.controls.iter().any(|c| c.region == "scrollbar.button-up"));
    // Kept: 3 button images + (3 gripper + 1 arrow) hover; the overlay and
    // the baked-label send button and the frame's hover piece are not.
    assert_eq!(r.tally.images, (7, 3 + 5 + 1 + 1));
    assert_eq!(r.tally.sets["Hover"], (2, 4));
    assert!(a
        .notice
        .contains("dropped ChatButtonSend Hover: no region slot"));
    assert!(a.notice.contains("titlebar.btn.close: the title button"));
}

#[test]
fn a_state_image_of_another_size_is_dropped_and_named() {
    let r = read(&tree_with_controls(17)).unwrap();
    let close = r
        .controls
        .iter()
        .find(|c| c.region == "titlebar.btn.close")
        .unwrap();
    assert!(!close
        .images
        .contains_key(&rekindle_types::dto::skin::RegionState::Disabled));
    assert!(r
        .tally
        .dropped
        .iter()
        .any(|(c, s, why)| c == "ButtonClose (titlebar.btn.close)"
            && *s == "Disabled"
            && why.contains("size differs")));
    let a = import(&tree_with_controls(17), "fixture").unwrap();
    validate::run(&a.canonical.bytes).unwrap();
}
