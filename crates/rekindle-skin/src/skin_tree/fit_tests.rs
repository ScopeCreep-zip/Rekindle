//! Plan SK2-S1d: the window sized to the pack's tree (overhang outsets, the
//! declared and the fitted smallest window), art-less tiles at their
//! component's size, parent-scoped names, rows of different views, the
//! INI's repeated position keys and Combat's lamp.

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{
    ActionKey, Anchor, LampKey, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey,
};

use super::roles::{role, Facts};
use super::{import, validate};
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

fn imported(xml: &[u8], parts: &[u8]) -> (WindowLayout, SkinTree) {
    let main = parse("main.xml", xml).unwrap();
    let parts = parse("parts.xml", parts).unwrap();
    let mut built = (frame(), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &mut built,
    )
    .unwrap();
    let tree = built.0.tree.clone().unwrap();
    (built.0, tree)
}

/// Total Chaos's `MainTitleBar` `OutdentLeft="1"` and a logo wider than the
/// frame's smallest size: the overhang joins the outsets, the window grows
/// to the declared `MinWidth`/`MinHeight` and then until the logo fits, and
/// every node then lies inside it (rule 3).
#[test]
fn the_window_fits_the_tree_and_its_overhang() {
    let (l, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="SCLogo" X="0" Y="60" Component="SCLogo"/>
</Tile>"#,
        br#"<Components>
<Component Name="MainWindow" MinWidth="100" MinHeight="90"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image>bar-10x50</Image></Images></Component>
<Component Name="SCLogo"><Images State="Normal"><Image>logo-174x59</Image></Images></Component>
</Components>"#,
    );
    assert_eq!(tree.outsets.left, 1);
    assert_eq!(l.outsets.left, 1);
    assert_eq!((l.min_width, l.min_height), (174, 119));
    let full = (i32::from(l.min_width) + 1, i32::from(l.min_height));
    let boxes = tree.boxes(full.0, full.1);
    assert_eq!(boxes[0].1, [0, 0, full.0, 50]);
    assert!(validate::check("t", &tree, &l, &BTreeMap::new()).is_ok());
}

/// Classic's `ResizeUp` (`ResizeX="100"`, no art) is its component's
/// `MinHeight="5"` tall: a real box, so rule 5 checks a real edge grip.
#[test]
fn an_art_less_tile_is_its_components_declared_size() {
    let (_, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="ResizeUp" ResizeX="100" Component="ResizeUp"/>
</Tile>"#,
        br#"<Components><Component Name="ResizeUp" Class="Tile" MinHeight="5"/></Components>"#,
    );
    let grip = &tree.root[0];
    assert_eq!(
        grip.role,
        NodeRole::Resize(rekindle_types::dto::skin_tree::Edge::Top)
    );
    assert_eq!(
        grip.y,
        NodeSpan::Fixed {
            at: Anchor::Start(0),
            size: 5
        }
    );
}

/// Alloy lays a `BvBk` in each view: the second name is scoped by its parent.
#[test]
fn a_repeated_name_is_scoped_by_its_parent() {
    let (_, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="Tabbed" ResizeX="100" ResizeY="100" Component="TabbedControl">
<Tile Name="BuddyView" ResizeX="100" ResizeY="100" Component="BuddyView"><Tile Name="BvBk" Component="BvBk"/></Tile>
<Tile Name="ServerView" ResizeX="100" ResizeY="100" Component="ServerView"><Tile Name="BvBk" Component="BvBk"/></Tile>
</Tile></Tile>"#,
        br#"<Components><Component Name="BvBk"><Images State="Normal"><Image>bk-4x4</Image></Images></Component></Components>"#,
    );
    let ids: Vec<&str> = tree
        .boxes(400, 400)
        .iter()
        .map(|(n, _)| n.id.as_str())
        .collect();
    assert!(
        ids.contains(&"BuddyView.BvBk") && ids.contains(&"ServerView.BvBk"),
        "{ids:?}"
    );
}

fn button(id: &str, key: ActionKey) -> SkinNode {
    SkinNode {
        id: id.to_owned(),
        role: NodeRole::Button(key),
        x: NodeSpan::Fixed {
            at: Anchor::Start(10),
            size: 30,
        },
        y: NodeSpan::Fixed {
            at: Anchor::Start(10),
            size: 30,
        },
        z: 0,
        art: None,
        text: None,
        shows_with: None,
        children: Vec::new(),
    }
}

fn row(id: &str, v: ViewKey, child: SkinNode) -> SkinNode {
    SkinNode {
        id: id.to_owned(),
        role: NodeRole::ButtonRow(v),
        x: NodeSpan::Stretch { start: 0, end: 0 },
        y: NodeSpan::Stretch { start: 0, end: 0 },
        z: 0,
        art: None,
        text: None,
        shows_with: Some(v),
        children: vec![child],
    }
}

/// Classic's `GameButton1`/`GameButton2` share a place in two views' rows
/// (never shown together); the same two in one row overlap (rule 4).
#[test]
fn rule_4_rows_of_different_views_may_share_a_place() {
    let mut l = frame();
    (l.min_width, l.min_height) = (200, 200);
    let view = |id: &str, v| SkinNode {
        role: NodeRole::View(v),
        children: Vec::new(),
        ..row(id, v, button(&format!("{id}x"), ActionKey::Chat))
    };
    let tree = |a: SkinNode, b: SkinNode| SkinTree {
        outsets: Z,
        root: vec![
            a,
            b,
            view("Fv", ViewKey::Friends),
            view("Sv", ViewKey::Servers),
        ],
    };
    let apart = tree(
        row("F", ViewKey::Friends, button("G1", ActionKey::Join)),
        row("S", ViewKey::Servers, button("G2", ActionKey::Join)),
    );
    validate::check("t", &apart, &l, &BTreeMap::new()).unwrap();
    let together = tree(
        row("F", ViewKey::Friends, button("G1", ActionKey::Join)),
        row("S", ViewKey::Friends, button("G2", ActionKey::Join)),
    );
    let e = validate::check("t", &together, &l, &BTreeMap::new()).unwrap_err();
    assert!(format!("{e:?}").contains("rule 4"), "{e:?}");
}

/// Combat's `DownloadBlink` is a tab control of blinkers: the files lamp.
#[test]
fn combats_download_blink_is_the_files_lamp() {
    let kids = ["WhiteBlinkNoAnimation".to_owned(), "RedBlinker".to_owned()];
    let f = Facts {
        has_art: false,
        class: Some("TabControl"),
        children: &kids,
    };
    assert_eq!(
        role("DownloadBlink", Some("DownloadBlink"), None, &f),
        Some(NodeRole::Lamp(LampKey::Files))
    );
}

/// XGlow `Xglow.ini:53-56`: `Window7.X=Top.RIGHT` then `X=-80` is one
/// position, the edge plus the offset; both entries are kept.
#[test]
fn an_ini_repeated_position_key_is_kept() {
    let ini = crate::import::xfire_ini::ini::parse(
        "x.ini",
        b"[MainWindow]\nWindow7.Name=XfireLogo\nWindow7.X=Top.RIGHT\nWindow7.X=-80\nName=a\nName=b\n",
    )
    .unwrap();
    let s = ini.section("MainWindow").unwrap();
    let xs = s.entries.iter().filter(|e| e.key == "Window7.X").count();
    let names = s.entries.iter().filter(|e| e.key == "Name").count();
    assert_eq!((xs, names), (2, 1));
}
