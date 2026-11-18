//! The importer on synthetic trees in the fan skins' late layouts (no
//! third-party art): one `components.xml` beside `skin.xml`, the chat
//! window's frame laid by its layout file, tile images named by path, and
//! the title buttons and scrollbar under the fan trees' names.

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::validate;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([40, 100, 160, 255]));
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

const COLOURS: &str = r#"<Color Name="ListBGCG">RGBA(22,22,22,0)</Color>
<Color Name="ListFriendTextCG">RGBA(183,184,184,0)</Color>"#;

/// The scrollbar's pieces, one bitmap each.
const SCROLL: [&str; 4] = ["ScrollUp", "ScrollDown", "ScrollLeft", "ScrollRight"];

/// A fan tree: lower-case files, `components.xml` holding `components`,
/// the chat layout `chatwindow.xml` holding `layout`; every bitmap `B*`
/// and every path under `art/` exists.
fn fan_tree(components: &str, layout: &str) -> Tree {
    let mut t = Tree::new();
    t.insert(
        "skin.xml".into(),
        br#"<XfireSkin><Include File="themes.xml" /><Include File="components.xml" />
<MainSkin><Include File="mainwindow.xml" /></MainSkin>
<ChatSkin><Include File="chatwindow.xml" /></ChatSkin></XfireSkin>"#
            .to_vec(),
    );
    let mut bitmaps = String::new();
    for n in ["BMIN", "BMINHOVER", "BLOGO", "BLOGOHOVER"]
        .into_iter()
        .chain(SCROLL)
    {
        let _ = write!(bitmaps, "<Bitmap name=\"{n}\">art/{n}.png</Bitmap>");
        t.insert(format!("art/{n}.png"), png(12, 12));
    }
    for (n, side) in [("tl", 6), ("t", 6), ("tr", 6), ("l", 6), ("m", 16)]
        .into_iter()
        .chain([("r", 6), ("bl", 6), ("b", 6), ("br", 6)])
    {
        t.insert(format!("art/{n}.png"), png(side, side));
    }
    t.insert(
        "themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors>{COLOURS}</Colors><Bitmaps>{bitmaps}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "components.xml".into(),
        format!("<Components>{components}</Components>").into_bytes(),
    );
    t.insert(
        "mainwindow.xml".into(),
        b"<Tile Name=\"Main\" Component=\"MainWindow\"></Tile>".to_vec(),
    );
    t.insert(
        "chatwindow.xml".into(),
        format!("<Tile Name=\"Chat\" Component=\"ChatWindow\">{layout}</Tile>").into_bytes(),
    );
    t.insert(
        "info.xml".into(),
        b"<SkinDescription><Name>Fan</Name><Author Type=\"Original\">A. Fan</Author></SkinDescription>"
            .to_vec(),
    );
    t
}

/// A title button, a logo push button (no slot) and a `Class="Scrollbar"`.
const CONTROLS: &str = r#"
<Component Name="Minimize" Class="PushButton"><Images State="Normal"><Image Type="middle">BMIN</Image></Images>
<Images State="Hover"><Image Type="middle">BMINHOVER</Image></Images></Component>
<Component Name="Logo" Class="PushButton"><Images State="Normal"><Image Type="middle">BLOGO</Image></Images>
<Images State="Hover"><Image Type="middle">BLOGOHOVER</Image></Images></Component>
<Component Name="ScrollbarA" Class="Scrollbar"><Images State="Normal">
<Image Type="ScrollUp">ScrollUp</Image><Image Type="ScrollDown">ScrollDown</Image>
<Image Type="ScrollLeft">ScrollLeft</Image><Image Type="ScrollRight">ScrollRight</Image></Images></Component>"#;

/// Onyx's and Space Cowboy's shape: one tile per piece, each component
/// naming its one image by path.
fn named_tiles() -> (String, String) {
    let mut components = String::from(CONTROLS);
    let mut layout = String::new();
    for (tile, file) in [
        ("TopLeft", "tl"),
        ("Top", "t"),
        ("TopRight", "tr"),
        ("LeftEdge", "l"),
        ("Middle", "m"),
        ("RightEdge", "r"),
        ("BottomLeft", "bl"),
        ("Bottom", "b"),
        ("BottomRight", "br"),
    ] {
        let _ = write!(
            components,
            "<Component Name=\"C{tile}\" Class=\"Tile\"><Images><Image Type=\"middle\">art/{file}.png</Image></Images></Component>"
        );
        let _ = write!(
            layout,
            "<Tile Name=\"{tile}\" Component=\"C{tile}\"></Tile>"
        );
    }
    (components, layout)
}

#[test]
fn named_tiles_and_fan_controls_import() {
    let (components, layout) = named_tiles();
    let t = fan_tree(&components, &layout);
    let r = read(&t).unwrap();
    let (img, insets) = r.frame.as_ref().unwrap();
    assert_eq!((img.width(), img.height()), (28, 28));
    assert_eq!((insets.top, insets.left), (6, 6));
    assert!(r.frame_source.contains("chatwindow.xml"));
    let regions: Vec<&str> = r.controls.iter().map(|c| c.region).collect();
    assert_eq!(
        regions,
        [
            "titlebar.btn.minimize",
            "scrollbar.button-up",
            "scrollbar.button-down",
            "scrollbar.button-left",
            "scrollbar.button-right"
        ]
    );
    let a = import(&t, "fan", None).unwrap();
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    assert!(a
        .notice
        .contains("window.frame: the nine-piece frame of the chat window layout"));
    assert!(a
        .notice
        .contains("titlebar.btn.minimize: the title button Minimize"));
    assert!(a.notice.contains("dropped Logo Hover: no region slot"));
}

/// xSLIMFIRE's shape: typed top and bottom rows, the sides on the tile
/// that resizes down the window (a fixed-height tile's sides are not).
#[test]
fn typed_rows_with_resizing_sides_import() {
    let components = format!(
        r#"{CONTROLS}
<Component Name="TitleBar" Class="Tile"><Images State="Normal"><Image Type="topleft">art/tl.png</Image><Image Type="top">art/t.png</Image><Image Type="topright">art/tr.png</Image></Images></Component>
<Component Name="Menu" Class="Tile"><Images><Image Type="left">art/m.png</Image><Image Type="right">art/m.png</Image></Images></Component>
<Component Name="MiddleRow" Class="Tile"><Images><Image Type="left">art/l.png</Image><Image Type="right">art/r.png</Image></Images></Component>
<Component Name="BaseBoard" Class="Tile"><Images><Image Type="bottomleft">art/bl.png</Image><Image Type="bottom">art/b.png</Image><Image Type="bottomright">art/br.png</Image></Images></Component>"#
    );
    let layout = r#"<Tile Name="ChatTitleBar" ResizeX="100" Component="TitleBar"></Tile>
<Tile Name="ChatMenu" ResizeX="100" Component="Menu"></Tile>
<Tile Name="ChatMiddleRow" ResizeY="100" Component="MiddleRow"></Tile>
<Tile Name="ChatBaseBoard" Component="BaseBoard"></Tile>"#;
    let r = read(&fan_tree(&components, layout)).unwrap();
    let (img, insets) = r.frame.as_ref().unwrap();
    assert!(r.frame_source.contains("vertically resizing tile"));
    // The 16 px menu pieces were not taken as sides.
    assert_eq!((img.width(), insets.left, insets.right), (18, 6, 6));
}

/// A layout that lays no complete shape gives no frame; the controls are
/// still read.
#[test]
fn an_incomplete_layout_has_no_frame() {
    let (components, layout) = named_tiles();
    let layout = layout.replace("<Tile Name=\"BottomRight\"", "<Tile Name=\"Corner\"");
    let r = read(&fan_tree(&components, &layout)).unwrap();
    assert!(r.frame.is_none());
    assert_eq!(r.controls.len(), 5);
}
