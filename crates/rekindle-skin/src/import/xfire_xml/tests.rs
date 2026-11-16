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
