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
    imported_rows(xml, parts, &[])
}

/// `xml` imported with its frame laid from the tiles `rows` (a row stack).
fn imported_rows(xml: &[u8], parts: &[u8], rows: &[&str]) -> (WindowLayout, SkinTree) {
    let main = parse("main.xml", xml).unwrap();
    let parts = parse("parts.xml", parts).unwrap();
    let mut built = (frame(), BTreeMap::new());
    let rows: Vec<String> = rows.iter().map(|r| (*r).to_owned()).collect();
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &rows,
        &mut built,
    )
    .unwrap();
    let tree = built.0.tree.clone().unwrap();
    (built.0, tree)
}

/// Total Chaos's `MainTitleBar` `OutdentLeft="1"` (a frame row) and a logo
/// wider than the frame's smallest size: the window holds the outdent, the
/// frame rectangle starts where the row does (lead decision 14), the window
/// grows to the declared `MinWidth`/`MinHeight` and then until the logo
/// fits, and every node then lies inside it (rule 3).
#[test]
fn the_window_fits_the_tree_and_its_overhang() {
    let (l, tree) = imported_rows(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="SCLogo" X="0" Y="60" Component="SCLogo"/>
</Tile>"#,
        br#"<Components>
<Component Name="MainWindow" MinWidth="100" MinHeight="90"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image>bar-10x50</Image></Images></Component>
<Component Name="SCLogo"><Images State="Normal"><Image>logo-174x59</Image></Images></Component>
</Components>"#,
        &["MainTitleBar"],
    );
    assert_eq!(tree.outsets.left, 1);
    assert_eq!(l.outsets.left, 0);
    // The frame rectangle holds the outdent: the logo's 174 px plus 1.
    assert_eq!((l.min_width, l.min_height), (175, 119));
    let full = (i32::from(l.min_width), i32::from(l.min_height));
    let boxes = tree.boxes(full.0, full.1);
    assert_eq!(boxes[0].1, [0, 0, full.0, 50]);
    assert!(validate::check("t", &tree, &l, &BTreeMap::new()).is_ok());
}

/// Plan SK2 lead decision 14: Total Chaos's frame rows (`components.xml`
/// `mainwindow.xml:2-37`), each `OutdentLeft="1"`, the `BottomBar` 66 px
/// tall under a `Content` `IndentBottom="61"`: the window holds the 1 px
/// outdent and the 5 px the bottom row reaches past the box, and the frame
/// rectangle is the rows' box (no outsets), so the frame art lies where the
/// tree's rows do.
#[test]
fn the_frame_rectangle_is_where_the_pack_places_its_rows() {
    let (l, tree) = imported_rows(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="Content" X="0" OutdentLeft="1" Y="MainTitleBar.BOTTOM" IndentBottom="61" ResizeX="100" ResizeY="100" Component="Content"/>
<Tile Name="BottomBar" X="0" OutdentLeft="1" Y="Content.BOTTOM" ResizeX="100" Component="BottomBar"/>
</Tile>"#,
        br#"<Components>
<Component Name="MainTitleBar"><Images State="Normal"><Image>bar-10x52</Image></Images></Component>
<Component Name="BottomBar"><Images State="Normal"><Image>bottom-10x66</Image></Images></Component>
</Components>"#,
        &["MainTitleBar", "Content", "BottomBar"],
    );
    let o = |i: Insets| (i.left, i.top, i.right, i.bottom);
    assert_eq!(o(tree.outsets), (1, 0, 0, 5));
    assert_eq!(o(l.outsets), (0, 0, 0, 0));
    let (w, h) = (i32::from(l.min_width), i32::from(l.min_height));
    let boxes = tree.boxes(w, h);
    // The rows span the window, the bottom row ends at its bottom edge.
    assert_eq!(boxes[0].1, [0, 0, w, 52]);
    assert_eq!(boxes[2].1, [0, h - 66, w, 66]);
}

/// Plan SK2 lead decision 14: Classic's `StatusWrapper` (`X="32" Y="40"
/// ResizeX="100" IndentRight="30"`, a `Group` with no art in the tree)
/// laid past the window's bottom never grows the window.
#[test]
fn an_art_less_wrapper_never_grows_the_window() {
    let (l, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="ResizeDown" JustY="BOTTOM" ResizeX="100" Component="ResizeDown">
<Tile Name="StatusWrapper" X="32" Y="40" ResizeX="100" IndentRight="30" Component="StatusWrapper"/>
</Tile></Tile>"#,
        br#"<Components>
<Component Name="MainWindow" MinWidth="100" MinHeight="90"/>
<Component Name="ResizeDown" Class="Tile" MinHeight="5"/>
<Component Name="StatusWrapper" Class="Tile" MinHeight="8"/>
</Components>"#,
    );
    assert_eq!(tree.outsets.bottom, 0);
    assert_eq!((l.outsets.bottom, l.min_height), (0, 90));
    let checked = validate::check("t", &tree, &l, &BTreeMap::new());
    assert!(checked.is_ok(), "{checked:?}");
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

/// Plan SK2 design §2.8 and N16: StealthX's `MenuButton` launches the
/// popup tile `MainMenuPopup` (`components.xml:549-562`, `popups.xml:2-89`);
/// the popup is the side rail, laid against the window, holding the menus,
/// the tabs, the per-view button rows and the chat buttons; the button is
/// its toggle.
#[test]
fn stealthxs_menu_button_launches_the_side_rail() {
    let main = parse(
        "main.xml",
        br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MenuButton" X="2" JustY="CENTER" Z="50" Component="MenuButton"/>
<Tile Name="TabbedControl" X="120" Y="10" IndentRight="10" IndentBottom="10" ResizeX="100" ResizeY="100" Component="TabbedControl">
<Tile Name="BuddyView" ResizeX="100" ResizeY="100" Component="BuddyView"/></Tile>
</Tile>"#,
    )
    .unwrap();
    let parts = parse(
        "components.xml",
        br#"<Components>
<Component Name="MainWindow" MinWidth="300" MinHeight="400"/>
<Component Name="MenuButton" Class="SkinLauncher"><Images State="Normal"><Image>menu-20x60</Image></Images><Popup SaveState="1">MainMenuPopup</Popup></Component>
<Component Name="MainMenuWindow" Class="Tile" DefaultWidth="114" MinWidth="114" MinHeight="360" BindX="false" BindY="true" ResizeY="100"/>
<Component Name="MenuBackground" Class="Tile"><Images><Image Type="middle">ground-10x10</Image></Images></Component>
<Component Name="MenuBar" Class="Tile" MinWidth="110" MinHeight="67"/>
<Component Name="XfireMenuText" Class="Menu"><Images State="Normal"><Image>xfire-80x20</Image></Images></Component>
<Component Name="TabHeader" Class="Tile" MinWidth="110" MinHeight="63"/>
<Component Name="TabNormalText" Class="Tab"><Images State="Normal"><Image>friends-80x20</Image></Images></Component>
<Component Name="ButtonRow" Class="Tile" MinWidth="110" MinHeight="84"/>
<Component Name="AddFriendButton" Class="PushButton"><Images State="Normal"><Image>add-100x20</Image></Images></Component>
<Component Name="ChatTab" Class="TabControl" MinWidth="110" MinHeight="21" ControlGroupId="5"/>
<Component Name="HostChatButton" Class="PushButton"><Images State="Normal"><Image>host-100x20</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let popups = parse(
        "popups.xml",
        br#"<Popups>
<Tile Name="MainMenuPopup" JustX="LEFT" X="0" Z="50" IndentTop="29" IndentBottom="28" Component="MainMenuWindow">
<Tile Name="MainMenuBackground" X="0" Y="0" ResizeX="100" ResizeY="100" Component="MenuBackground">
<Tile Name="MenuBar" X="1" Y="10" Component="MenuBar"><Tile Name="XfireMenuText" X="1" Y="4" Component="XfireMenuText"/></Tile>
<Tile Name="TabHeader" X="1" Y="MenuBar.BOTTOM+20" Component="TabHeader"><Tile Name="TabNormalText" X="1" Y="0" Component="TabNormalText"/></Tile>
<Tile Name="ButtonArea" X="2" Y="TabHeader.BOTTOM+16" Component="TabbedControl">
<Tile Name="FriendsButtons" Component="ButtonRow"><Tile Name="AddFriendButton" Component="AddFriendButton"/></Tile></Tile>
<Tile Name="InfoJoinChatTab" X="2" Y="TabHeader.BOTTOM+100" Component="ChatTab"><Tile Name="HostChatButton" Component="HostChatButton"/></Tile>
</Tile></Tile>
<Tile Name="MainInfoViewPopup" JustX="LEFT" X="0" Component="MainInfoWindow"/>
</Popups>"#,
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
    let tree = built.0.tree.unwrap();
    let mut roles = Vec::new();
    let mut stack: Vec<(&SkinNode, usize)> = tree.root.iter().rev().map(|n| (n, 0)).collect();
    while let Some((n, d)) = stack.pop() {
        roles.push((d, n.id.as_str(), n.role));
        stack.extend(n.children.iter().rev().map(|c| (c, d + 1)));
    }
    let want = [
        (0, "MenuButton", NodeRole::SideRailToggle),
        (0, "TabbedControl", NodeRole::Views),
        (1, "BuddyView", NodeRole::View(ViewKey::Friends)),
        (0, "MainMenuPopup", NodeRole::SideRail),
        (1, "MainMenuBackground", NodeRole::Decor),
        (2, "MenuBar", NodeRole::MenuBar),
        (
            3,
            "XfireMenuText",
            NodeRole::Menu(rekindle_types::dto::skin_tree::MenuKey::First),
        ),
        (2, "TabHeader", NodeRole::TabStrip),
        (3, "TabNormalText", NodeRole::Tab(ViewKey::Friends)),
        (2, "ButtonArea", NodeRole::ButtonBar),
        (3, "FriendsButtons", NodeRole::ButtonRow(ViewKey::Friends)),
        (4, "AddFriendButton", NodeRole::Button(ActionKey::AddFriend)),
        (2, "InfoJoinChatTab", NodeRole::ButtonBar),
        (3, "HostChatButton", NodeRole::Button(ActionKey::HostChat)),
    ];
    assert_eq!(roles, want);
    let rail = tree.root.iter().find(|n| n.id == "MainMenuPopup").unwrap();
    assert_eq!(
        (rail.x, rail.y),
        (
            NodeSpan::Fixed {
                at: Anchor::Start(0),
                size: 114
            },
            NodeSpan::Stretch { start: 29, end: 28 }
        )
    );
}

/// Plan SK2-S1o: StealthX's `XfireMenuText` (`X="1" Y="4"
/// IndentLeft="27"`, fixed) holds `FileMenu` (`ResizeX="100"
/// OutdentLeft="27"`), `popups.xml:5-6`: the fixed tile's indent is its
/// client inset, so the menu art starts at the text tile's own left edge,
/// never 25 px left of the window.
#[test]
fn a_fixed_tiles_indent_is_its_client_inset() {
    let (l, tree) = imported(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MenuBar" X="1" Y="10" Component="MenuBar">
<Tile Name="XfireMenuText" X="1" Y="4" IndentLeft="27" Component="XfireMenuText">
<Tile Name="FileMenu" X="0" Y="-4" ResizeX="100" ResizeY="100" OutdentLeft="27" Component="FileMenu"/>
</Tile></Tile></Tile>"#,
        br#"<Components>
<Component Name="MenuBar"><Images State="Normal"><Image>bar-110x60</Image></Images></Component>
<Component Name="XfireMenuText"><Images State="Normal"><Image>text-80x16</Image></Images></Component>
<Component Name="FileMenu"><Images State="Normal"><Image>menu-80x20</Image></Images></Component>
</Components>"#,
    );
    assert_eq!(tree.outsets.left, 0);
    let (w, h) = (i32::from(l.min_width), i32::from(l.min_height));
    let boxes = tree.boxes(w, h);
    assert_eq!(boxes[2].1, [2, 10, 80, 20]);
}

/// Plan SK2-S1o: StealthX's nine-piece frame is `MainBackground`'s set,
/// laid at `X="112"` (`mainwindow.xml:2`) right of the rail: the frame
/// rectangle starts there, so the frame art and the views content agree.
#[test]
fn a_frame_tile_inside_the_window_starts_the_frame_rectangle() {
    let (l, tree) = imported_rows(
        br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MainBackground" X="112" Y="0" ResizeX="100" ResizeY="100" Component="MainBackground">
<Tile Name="TabbedControl" X="26" Y="35" IndentRight="25" IndentBottom="101" ResizeX="100" ResizeY="100" Component="TabbedControl">
<Tile Name="BuddyView" ResizeX="100" ResizeY="100" Component="BuddyView"/></Tile>
</Tile></Tile>"#,
        br#"<Components>
<Component Name="MainWindow" MinWidth="400" MinHeight="400"/>
</Components>"#,
        &["MainBackground"],
    );
    let o = |i: Insets| (i.left, i.top, i.right, i.bottom);
    assert_eq!(o(tree.outsets), (0, 0, 0, 0));
    assert_eq!(o(l.outsets), (112, 0, 0, 0));
    assert_eq!(o(l.content), (26, 35, 25, 101));
    assert_eq!(l.min_width, 400 - 112);
}
