//! Plan SK2 lead decision 15: the side rail's nine-piece ground laid as
//! decoration tiles, so the silhouette holds the rail.

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, SkinWindow, WindowLayout};

use super::import;
use crate::import::tree::parse;

const Z: Insets = Insets {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
};

fn frame() -> WindowLayout {
    WindowLayout {
        outsets: Z,
        content: Z,
        min_width: 30,
        min_height: 20,
        tiles: Vec::new(),
        title: None,
        tree: None,
    }
}

/// A solid image sized by its name's `-WxH` suffix.
fn piece(path: &str) -> RgbaImage {
    let (w, h) = path
        .rsplit_once('-')
        .and_then(|(_, d)| d.split_once('x'))
        .unwrap();
    RgbaImage::from_pixel(
        w.parse().unwrap(),
        h.parse().unwrap(),
        Rgba([10, 20, 30, 255]),
    )
}

/// Plan SK2 lead decision 15: StealthX's rail is in the silhouette. Its
/// ground `MainMenuBackground` (`Component="MenuBackground"`, a nine-piece
/// set, `components.xml:26-40`) is laid over the rail's box (x 0, 114 wide,
/// 29 px below the top to 28 px above the bottom) as three decoration tiles
/// in the whole window: the top row, the middle band repeated, the bottom.
#[test]
fn stealthxs_rail_ground_is_laid_as_decoration() {
    let main = parse(
        "main.xml",
        br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MenuButton" X="2" JustY="CENTER" Z="50" Component="MenuButton"/>
<Tile Name="TabbedControl" X="120" Y="10" IndentRight="10" IndentBottom="10" ResizeX="100" ResizeY="100" Component="TabbedControl"/>
</Tile>"#,
    )
    .unwrap();
    let images = [
        "tl-16x6", "t-13x6", "tr-13x6", "l-16x24", "m-28x34", "r-13x24", "bl-16x7", "b-13x7",
        "br-13x7",
    ]
    .iter()
    .zip(crate::import::xfire_xml::bitmap::NINE)
    .fold(String::new(), |mut s, (i, t)| {
        use std::fmt::Write;
        let _ = write!(s, r#"<Image Type="{t}">{i}</Image>"#);
        s
    });
    let parts = parse(
        "components.xml",
        format!(
            r#"<Components>
<Component Name="MainWindow" MinWidth="300" MinHeight="400"/>
<Component Name="MenuButton" Class="SkinLauncher"><Images State="Normal"><Image>menu-20x60</Image></Images><Popup SaveState="1">MainMenuPopup</Popup></Component>
<Component Name="MainMenuWindow" Class="Tile" DefaultWidth="114" MinWidth="114" MinHeight="360" BindX="false" BindY="true" ResizeY="100"/>
<Component Name="MenuBackground" Class="Tile"><Images>{images}</Images></Component>
</Components>"#
        )
        .as_bytes(),
    )
    .unwrap();
    let popups = parse(
        "popups.xml",
        br#"<Popups><Tile Name="MainMenuPopup" JustX="LEFT" X="0" Z="50" IndentTop="29" IndentBottom="28" Component="MainMenuWindow">
<Tile Name="MainMenuBackground" X="0" Y="0" ResizeX="100" ResizeY="100" Component="MenuBackground"/></Tile></Popups>"#,
    )
    .unwrap();
    let mut built = (frame(), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &main,
        &[parts, popups],
        |path, _| Ok(piece(path)),
        &[],
        &mut built,
    )
    .unwrap();
    use rekindle_types::dto::skin::TileAxis::{End, Repeat, Start};
    let got: Vec<_> = built
        .0
        .tiles
        .iter()
        .map(|t| {
            (
                t.image.as_str(),
                t.width,
                t.height,
                t.x,
                t.y,
                t.overhang,
                t.decoration,
            )
        })
        .collect();
    let at = Start { offset: 0 };
    assert_eq!(
        got,
        [
            (
                "assets/frame/main/rail-top.png",
                114,
                6,
                at,
                Start { offset: 29 },
                true,
                Some(50)
            ),
            (
                "assets/frame/main/rail-middle.png",
                114,
                408,
                at,
                Repeat { start: 35, end: 35 },
                true,
                Some(50)
            ),
            (
                "assets/frame/main/rail-bottom.png",
                114,
                7,
                at,
                End { offset: 28 },
                true,
                Some(50)
            ),
        ]
    );
    for t in &built.0.tiles {
        assert!(built.1.contains_key(&t.image), "{} has its PNG", t.image);
    }
}
