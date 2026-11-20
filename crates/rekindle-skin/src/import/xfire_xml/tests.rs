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

const PASSING: &str = r#"<Color Name="ListBGCG">RGBA(16,16,16,0)</Color>
<Color Name="ListFriendTextCG">RGBA(183,184,184,0)</Color>
<Color Name="XIGCaretCG">RGBA(255,255,255,0)</Color>
<Color Name="SideCG">RGBA(30,-30,0,0)</Color>"#;

#[test]
fn imports_validates_and_is_byte_identical() {
    let t = tree(PASSING);
    let a = import(&t, "fixture", None).unwrap();
    let b = import(&t, "fixture", None).unwrap();
    assert_eq!(a, b);
    // #101010 is every window's ground too (`bg.dark` follows `bg.primary`,
    // plan E7.7.7 e777n): white on the online chip (`status.online` 60% over
    // it) is 4.44:1, under D4's 4.5:1, and the walk records darkening the
    // skin's `bg.primary` to #0d0d0d.
    let c = a.changes.first().unwrap();
    assert_eq!(
        (c.token.as_str(), c.rule.as_str(), c.ground.as_str()),
        ("bg.primary", "D4", "status.online 60.0% over bg.dark")
    );
    assert_eq!(
        (c.original, c.new, c.before_x100, c.required_x100),
        ([16, 16, 16], [13, 13, 13], 444, 450)
    );
    let again = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(again.bytes, a.canonical.bytes);
    assert!(a
        .notice
        .contains("XIGCaretCG RGBA(255,255,255,0) -> dropped"));
    assert!(a.notice.contains("triggers: 1"));
}

#[test]
fn non_zero_alpha_is_refused() {
    let t = tree(&PASSING.replace("RGBA(16,16,16,0)", "RGBA(16,16,16,1)"));
    assert!(matches!(
        import(&t, "fixture", None),
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
    let a = import(&t, "fixture", None).unwrap();
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
    let third = import(&t, "fixture", None).unwrap();
    assert!(third.notice.contains("third-party work"));
    assert!(!third.notice.contains("2026-10-15"));
    t.insert(
        "Info.xml".into(),
        br#"<SkinDescription><Author Type="Contributor">Someone</Author><Author Type="Original">Xfire, Inc.</Author><Name>Fixture</Name></SkinDescription>"#.to_vec(),
    );
    let own = import(&t, "fixture", None).unwrap();
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
        "<Components><Component Name=\"ButtonClose\" Class=\"PushButton\"><Images State=\"Normal\"><Image Type=\"Middle\">close_n</Image></Images><Images State=\"Hover\"><Image Type=\"Middle\">close_h</Image></Images><Images State=\"Down\"><Image Type=\"Middle\">close_d</Image></Images><Images State=\"Disabled\"><Image Type=\"Middle\">close_x</Image></Images></Component><Component Name=\"XfireScrollbar\" Class=\"Scrollbar\">{}{}</Component><Component Name=\"ChatButtonJoin\" Class=\"PushButton\"><Images State=\"Normal\"><Image Type=\"Middle\">send_h</Image></Images><Images State=\"Hover\"><Image Type=\"Middle\">send_h</Image></Images></Component></Components>",
        state("Normal", "n"),
        state("Hover", "h")
    );
    t.insert("Components/Common.xml".into(), common.into_bytes());
    t
}

#[test]
fn title_buttons_and_the_scrollbar_keep_their_state_images() {
    let a = import(&tree_with_controls(18), "fixture", None).unwrap();
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
    // the baked-label join button and the frame's hover piece are not.
    assert_eq!(r.tally.images, (7, 3 + 5 + 1 + 1));
    assert_eq!(r.tally.sets["Hover"], (2, 4));
    assert!(a
        .notice
        .contains("dropped ChatButtonJoin Hover: no region slot"));
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
    let a = import(&tree_with_controls(17), "fixture", None).unwrap();
    validate::run(&a.canonical.bytes).unwrap();
}

/// Classic v3's shape (plan E7.7.3b), synthetic: lower-case files,
/// attributes in either case, attribute colours (one with no `alpha`), and
/// the chat frame over four components of `xfirecomponents.xml`.
fn v3_tree(colours: &str) -> Tree {
    let mut t = Tree::new();
    t.insert(
        "skin.xml".into(),
        br#"<!DOCTYPE XfireSkin SYSTEM "x.dtd"><XfireSkin><Include File="xfirethemes.xml" /><Include File="xfirecomponents.xml" /></XfireSkin>"#.to_vec(),
    );
    t.insert(
        "info.xml".into(),
        br#"<SkinDescription><Author Type="original">Xfire, Inc.</Author><Author Type="contributor">Someone</Author><Name>V3</Name><Version>3</Version></SkinDescription>"#.to_vec(),
    );
    let mut bitmaps = String::new();
    for n in ["tl", "t", "tr", "l", "r", "bl", "b", "br"] {
        let _ = write!(
            bitmaps,
            "<Bitmap name=\"{}\">frame/{n}.png</Bitmap>",
            n.to_uppercase()
        );
        t.insert(
            format!("frame/{n}.png"),
            png(4, 4, Rgba([40, 100, 160, 255])),
        );
    }
    t.insert(
        "xfirethemes.xml".into(),
        format!("<Themes><Theme name=\"default\"><Colors>{colours}</Colors><Bitmaps>{bitmaps}</Bitmaps></Theme><Theme name=\"other\"/></Themes>").into_bytes(),
    );
    t.insert(
        "xfirecomponents.xml".into(),
        br#"<Components>
<Component Name="ChatTitleBar" Class="Tile"><Images state="Normal" Color="ChatMainCG"><Image Type="topleft">TL</Image><Image Type="top">T</Image><Image Type="topright">TR</Image></Images>
<Images state="Disabled" Color="ChatMainDeactivatedCG"><Image Type="topleft">TL</Image></Images></Component>
<Component Name="UserNameField" Class="Tile"><Images><Image type="left">L</Image><Image type="middle">T</Image></Images></Component>
<Component Name="ChatLeftOuterEdge" Class="Tile"><Images Color="ChatMainCG"><Image Type="top">TL</Image><Image Type="middle">L</Image></Images></Component>
<Component Name="ChatRightOuterEdge" Class="Tile"><Images Color="ChatMainCG"><Image type="middle">R</Image></Images></Component>
<Component name="ChatBaseBoard" Class="Tile"><Images><Image Type="bottomleft">BL</Image><Image Type="bottom">B</Image><Image Type="bottomright">BR</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t
}

const V3_COLOURS: &str = r#"<Color name="BuddyViewBG" red="22" green="22" blue="22" alpha="0" />
<Color Name="BuddyViewFriendText" red="183" green="184" blue="184" />
<Color name="ChatMainCG" red="30" green="-30" blue="0" alpha="0" />
<Color name="AdAreaCG">RGBA(8,16,32,0)</Color>"#;

#[test]
fn classic_v3_layout_imports() {
    let r = read(&v3_tree(V3_COLOURS)).unwrap();
    assert_eq!(r.layout, Layout::ClassicV3);
    assert!(r.is_xfire_own(), "Type=\"original\" names Xfire");
    assert_eq!(r.info.get("Version").map(String::as_str), Some("3"));
    assert_eq!(r.tokens["bg.primary"], walk::color([22, 22, 22]));
    assert_eq!(r.tokens["text.base"], walk::color([183, 184, 184]));
    let (img, insets) = r.frame.unwrap();
    assert_eq!(
        (insets.top, insets.left, insets.bottom, insets.right),
        (4, 4, 4, 4)
    );
    // The top row and the sides carry ChatMainCG (+30, -30, 0); the base
    // board does not; each piece's #FF00FF pixel is keyed; the middle is
    // transparent.
    assert_eq!(img.get_pixel(5, 1).0, [70, 70, 160, 255]);
    assert_eq!(img.get_pixel(1, 5).0, [70, 70, 160, 255]);
    assert_eq!(img.get_pixel(5, img.height() - 2).0, [40, 100, 160, 255]);
    assert_eq!(img.get_pixel(4, 0).0, [0, 0, 0, 0]);
    assert_eq!(img.get_pixel(4, 4).0, [0, 0, 0, 0]);
    let a = import(&v3_tree(V3_COLOURS), "fixture", None).unwrap();
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    assert!(a.notice.contains("Xfire, Inc.'s own work"), "{}", a.notice);
    assert!(a
        .notice
        .contains("BuddyViewBG RGBA(22,22,22,0) -> token bg.primary"));
    assert!(a.notice.contains("AdAreaCG RGBA(8,16,32,0) -> dropped"));
}

#[test]
fn classic_v3_alpha_and_missing_pieces() {
    let t = v3_tree(&V3_COLOURS.replace("blue=\"22\" alpha=\"0\"", "blue=\"22\" alpha=\"5\""));
    assert!(matches!(
        read(&t),
        Err(ImportError::NonZeroAlpha { alpha: 5, .. })
    ));
    let mut t = v3_tree(V3_COLOURS);
    let comps = String::from_utf8(t["xfirecomponents.xml"].clone()).unwrap();
    t.insert(
        "xfirecomponents.xml".into(),
        comps
            .replace("Type=\"bottomright\"", "Type=\"corner\"")
            .into_bytes(),
    );
    assert!(read(&t).unwrap().frame.is_none());
}

#[test]
fn a_credit_names_xfire_when_info_does_not() {
    let credit = crate::import::XfireCredit::new(
        "https://archive.org/details/XFireSkins",
        "An official skin created by Xfire.",
    )
    .unwrap();
    let a = import(&tree(PASSING), "fixture", Some(&credit)).unwrap();
    assert!(
        a.notice.contains("stated outside the\n  skin's files"),
        "{}",
        a.notice
    );
    assert!(a.notice.contains("archive.org/details/XFireSkins"));
}

/// Plan E7.7.3d: `Info.xml`'s authors by type, its `Website` and a root
/// README reach the manifest's credits, as the skin states them.
#[test]
fn info_xml_credits_reach_the_manifest() {
    use rekindle_types::dto::skin::OriginalLicence;
    let mut t = tree(PASSING);
    t.insert(
        "Info.xml".into(),
        br#"<SkinDescription><Author Type="original">Ann</Author><Author Type="contributor">Bo</Author><Author Type="Contributor">Ann</Author><Website>http://skins.example/</Website><Name>Fixture</Name></SkinDescription>"#.to_vec(),
    );
    t.insert("ReadMe.txt".into(), b"Share freely.\r\n".to_vec());
    let i = import(&t, "fixture", None).unwrap();
    let m = crate::manifest::parse(&crate::package::read(&i.canonical.bytes).unwrap()).unwrap();
    let c = m.credits.expect("credits");
    assert_eq!(
        (c.authors, c.contributors),
        (vec!["Ann".to_owned()], vec!["Bo".to_owned()])
    );
    assert_eq!(c.url.as_deref(), Some("http://skins.example/"));
    assert_eq!(c.licence, OriginalLicence::Text("Share freely.".into()));
    let bare = import(&tree(PASSING), "fixture", None).unwrap();
    let m = crate::manifest::parse(&crate::package::read(&bare.canonical.bytes).unwrap()).unwrap();
    let c = m.credits.expect("credits");
    assert!(c.authors.is_empty() && c.url.is_none());
    assert_eq!(c.licence, OriginalLicence::NoneStated);
}
