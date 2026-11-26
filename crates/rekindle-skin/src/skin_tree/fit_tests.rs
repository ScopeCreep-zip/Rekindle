//! Plan SK2-S1d: the window sized to the pack's tree (overhang outsets, the
//! declared and the fitted smallest window), art-less tiles at their
//! component's size, parent-scoped names, rows of different views, the
//! INI's repeated position keys and Combat's lamp.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{Insets, PopupAnchor, PopupEdge, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_action::{NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{
    Anchor, LampKey, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey,
};

use super::roles::{role, Facts, Role};
use super::test_support::{frame, piece, Z};
use super::{import, validate};
use crate::import::tree::parse;

fn imported(xml: &[u8], parts: &[u8]) -> (WindowLayout, SkinTree) {
    imported_rows(xml, parts, &[])
}

/// `xml` imported with its frame laid from the tiles `rows` (a row stack).
fn imported_rows(xml: &[u8], parts: &[u8], rows: &[&str]) -> (WindowLayout, SkinTree) {
    let (l, tree, _) = imported_art(xml, parts, rows);
    (l, tree)
}

/// [`imported_rows`] with the images the import wrote, which every imaged
/// node's art names (plan SK2 lead decision 18).
fn imported_art(
    xml: &[u8],
    parts: &[u8],
    rows: &[&str],
) -> (WindowLayout, SkinTree, BTreeMap<String, Vec<u8>>) {
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
    (built.0, tree, built.1)
}

/// Total Chaos's `MainTitleBar` `OutdentLeft="1"` (a frame row) and a logo
/// wider than the frame's smallest size: the outdent never grows the window
/// (lead decision 29, replacing 14), the row's art is cut at the window's
/// edge, its middle repeat starting a column later (no start cap), the window
/// grows to the declared `MinWidth`/`MinHeight` and then until the logo
/// fits, and every node then lies inside it (rule 3). The bar and the logo
/// carry their art (lead decision 18: every imaged node does), so the tree
/// is checked against the images the import wrote.
#[test]
fn the_window_fits_the_tree_and_its_overhang() {
    let (l, tree, images) = imported_art(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="SCLogo" X="0" Y="60" Component="SCLogo"/>
</Tile>"#,
        br#"<Components>
<Component Name="MainWindow" MinWidth="100" MinHeight="90"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="Middle">bar-10x50</Image></Images></Component>
<Component Name="SCLogo"><Images State="Normal"><Image Type="Middle">logo-174x59</Image></Images></Component>
</Components>"#,
        &["MainTitleBar"],
    );
    assert_eq!(tree.outsets.left, 0);
    assert_eq!(l.outsets.left, 0);
    // The logo's 174 px; the outdent's column is cut, not added.
    assert_eq!((l.min_width, l.min_height), (174, 119));
    let bar = tree.root.iter().find(|n| n.id == "MainTitleBar").unwrap();
    assert_eq!(bar.x, NodeSpan::Stretch { start: 0, end: 0 });
    let full = (i32::from(l.min_width), i32::from(l.min_height));
    let boxes = tree.boxes(full.0, full.1);
    assert_eq!(boxes[0].1, [0, 0, full.0, 50]);
    let logo = tree.root.iter().find(|n| n.id == "SCLogo").unwrap();
    assert!(logo.art.is_some(), "a decor logo carries its art");
    let checked = validate::check("t", &tree, &l, &images);
    assert!(checked.is_ok(), "{checked:?}");
}

/// Plan SK2 lead decision 27 (correcting 14): Total Chaos's frame rows
/// (`mainwindow.xml:2-37`), each `OutdentLeft="1"`, the `BottomBar` 66 px
/// tall under a `Content` `IndentBottom="61"`: the window holds neither the
/// 1 px outdent (lead decision 29: each row's left cap loses its first
/// column, the original 470 px wide) nor the 5 px the bottom row reaches
/// past the box; a centred child keeps its place; the row is
/// clipped on its fixed axis to its top 61 rows at the window's bottom edge
/// (the original, `skinref/total-chaos/buddy-list-1.png`), and its one-row
/// set is trimmed piece by piece, each kept in its slot.
#[test]
fn the_frame_rectangle_is_where_the_pack_places_its_rows() {
    let (l, tree, images) = imported_art(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="Content" X="0" OutdentLeft="1" Y="MainTitleBar.BOTTOM" IndentBottom="61" ResizeX="100" ResizeY="100" Component="Content"/>
<Tile Name="BottomBar" X="0" OutdentLeft="1" Y="Content.BOTTOM" ResizeX="100" Component="BottomBar">
<Tile Name="BottomCenter" JustX="CENTER" X="0" Y="0" Component="BottomCenter"/>
<Tile Name="TabButtonArea" X="0" Y="0" ResizeX="100" Component="TabButtonArea"/>
</Tile>
</Tile>"#,
        br#"<Components>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="Middle">bar-10x52</Image></Images></Component>
<Component Name="Content"><Images State="Normal"><Image Type="Middle">ct-10x10</Image></Images></Component>
<Component Name="BottomCenter"><Images State="Normal"><Image Type="Middle">bc-40x20</Image></Images></Component>
<Component Name="TabButtonArea"><Images State="Normal"><Image Type="Middle">tb-10x20</Image></Images></Component>
<Component Name="BottomBar"><Images State="Normal"><Image Type="left">bl-161x66</Image><Image Type="middle">bm-37x66</Image><Image Type="right">br-161x66</Image></Images></Component>
</Components>"#,
        &["MainTitleBar", "Content", "BottomBar"],
    );
    let o = |i: Insets| (i.left, i.top, i.right, i.bottom);
    assert_eq!(o(tree.outsets), (0, 0, 0, 0));
    assert_eq!(o(l.outsets), (0, 0, 0, 0));
    let (w, h) = (i32::from(l.min_width), i32::from(l.min_height));
    let boxes = tree.boxes(w, h);
    // The rows span the window, the bottom row's top 61 rows end at its
    // bottom edge.
    assert_eq!(boxes[0].1, [0, 0, w, 52]);
    assert_eq!(boxes[2].1, [0, h - 61, w, 61]);
    // Centred in the 1 + w px box from −1: (w + 1 − 40) / 2 − 1.
    let centre = boxes
        .iter()
        .find(|(n, _)| n.id == "BottomCenter")
        .unwrap()
        .1;
    assert_eq!(centre[0], (w + 1 - 40).div_euclid(2) - 1, "w = {w}");
    // A stretched child flush with the row (`TabButtonArea`,
    // `mainwindow.xml:43`) is clipped to the row in turn: it starts at the
    // window's edge, not one column before it.
    let area = boxes.iter().find(|(n, _)| n.id == "TabButtonArea").unwrap();
    assert_eq!(area.0.x, NodeSpan::Stretch { start: 0, end: 0 });
    assert_eq!((area.1[0], area.1[2]), (0, w));
    let bottom = tree.root.iter().find(|n| n.id == "BottomBar").unwrap();
    let set = &bottom.art.as_ref().unwrap().normal;
    let dims = |p: &str| {
        let png = &images[p];
        let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
        (be(16), be(20))
    };
    let got: Vec<_> = set
        .iter()
        .map(|(s, p)| (*s, p.width, p.height, dims(&p.image)))
        .collect();
    use rekindle_types::dto::skin_tree::PieceSlot as S;
    assert_eq!(
        got,
        [
            (S::Left, 160, 61, (160, 61)),
            (S::Middle, 37, 61, (37, 61)),
            (S::Right, 161, 61, (161, 61)),
        ]
    );
    let checked = validate::check("t", &tree, &l, &images);
    assert!(checked.is_ok(), "{checked:?}");
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
    assert_eq!(grip.role, NodeRole::Button);
    assert_eq!(
        grip.resize_edge(),
        Some(rekindle_types::dto::skin_tree::Edge::Top)
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
        br#"<Components><Component Name="BvBk"><Images State="Normal"><Image Type="Middle">bk-4x4</Image></Images></Component></Components>"#,
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

fn button(id: &str, key: SkinAction) -> SkinNode {
    SkinNode {
        id: id.to_owned(),
        role: NodeRole::Button,
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
        label: None,
        inset: None,
        target: None,
        fit: None,
        shows_with: None,
        control_group: None,
        disabled_by: None,
        triggers: vec![NodeTrigger {
            on: SkinEvent::Click,
            run: key,
        }],
        children: Vec::new(),
    }
}

fn row(id: &str, v: ViewKey, child: SkinNode) -> SkinNode {
    SkinNode {
        role: NodeRole::ButtonRow(v),
        x: NodeSpan::Stretch { start: 0, end: 0 },
        y: NodeSpan::Stretch { start: 0, end: 0 },
        z: 0,
        shows_with: Some(v),
        triggers: Vec::new(),
        children: vec![child],
        ..button(id, SkinAction::Minimize)
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
        ..row(id, v, button(&format!("{id}x"), SkinAction::ChatFriend))
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
        row("F", ViewKey::Friends, button("G1", SkinAction::JoinFriend)),
        row("S", ViewKey::Servers, button("G2", SkinAction::JoinFriend)),
    );
    validate::check("t", &apart, &l, &BTreeMap::new()).unwrap();
    let together = tree(
        row("F", ViewKey::Friends, button("G1", SkinAction::JoinFriend)),
        row("S", ViewKey::Friends, button("G2", SkinAction::JoinFriend)),
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
        Some(Role::Shows(NodeRole::Lamp(LampKey::Files)))
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
/// its toggle. The rail is a popup tree of its own (design §2.3, one layout
/// model): it is laid only as the window's popup, never in its tree; the
/// popup's box is the rail (like Classic's info popup, the popup tile is
/// the popup window), its tree the tile's children.
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
<Component Name="MenuButton" Class="SkinLauncher"><Triggers><Trigger Type="OnClick">LaunchSkin</Trigger></Triggers><Images State="Normal"><Image Type="Middle">menu-20x60</Image></Images><Popup SaveState="1">MainMenuPopup</Popup></Component>
<Component Name="MainMenuWindow" Class="Tile" DefaultWidth="114" MinWidth="114" MinHeight="360" BindX="false" BindY="true" ResizeY="100"/>
<Component Name="MenuBackground" Class="Tile"><Images><Image Type="middle">ground-10x10</Image></Images></Component>
<Component Name="MenuBar" Class="Tile" MinWidth="110" MinHeight="67"/>
<Component Name="XfireMenuText" Class="Menu"><Images State="Normal"><Image Type="Middle">xfire-80x20</Image></Images></Component>
<Component Name="TabHeader" Class="Tile" MinWidth="110" MinHeight="63"/>
<Component Name="TabNormalText" Class="Tab"><Images State="Normal"><Image Type="Middle">friends-80x20</Image></Images></Component>
<Component Name="ButtonRow" Class="Tile" MinWidth="110" MinHeight="84"/>
<Component Name="AddFriendButton" Class="PushButton"><Images State="Normal"><Image Type="Middle">add-100x20</Image></Images></Component>
<Component Name="ChatTab" Class="TabControl" MinWidth="110" MinHeight="21" ControlGroupId="5"/>
<Component Name="HostChatButton" Class="PushButton"><Images State="Normal"><Image Type="Middle">host-100x20</Image></Images></Component>
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
    let depths = |root: &[SkinNode]| {
        let mut roles = Vec::new();
        let mut stack: Vec<(&SkinNode, usize)> = root.iter().rev().map(|n| (n, 0)).collect();
        while let Some((n, d)) = stack.pop() {
            roles.push((d, n.id.clone(), n.role));
            stack.extend(n.children.iter().rev().map(|c| (c, d + 1)));
        }
        roles
    };
    let own = |w: &[(usize, &str, NodeRole)]| -> Vec<(usize, String, NodeRole)> {
        w.iter()
            .map(|(d, id, r)| (*d, (*id).to_owned(), *r))
            .collect()
    };
    assert_eq!(
        depths(&tree.root),
        own(&[
            (0, "MenuButton", NodeRole::Button),
            (0, "TabbedControl", NodeRole::Views),
            (1, "BuddyView", NodeRole::View(ViewKey::Friends)),
        ])
    );
    let rail = &built.0.popups["MainMenuPopup"];
    let want = [
        (0, "MainMenuBackground", NodeRole::Decor),
        (1, "MenuBar", NodeRole::MenuBar),
        (2, "XfireMenuText", NodeRole::MenuTitle),
        (1, "TabHeader", NodeRole::TabStrip),
        (2, "TabNormalText", NodeRole::Tab(ViewKey::Friends)),
        (1, "ButtonArea", NodeRole::ButtonBar),
        (2, "FriendsButtons", NodeRole::ButtonRow(ViewKey::Friends)),
        (3, "AddFriendButton", NodeRole::Button),
        (1, "InfoJoinChatTab", NodeRole::ButtonBar),
        (2, "HostChatButton", NodeRole::Button),
    ];
    assert_eq!(depths(&rail.tree.root), own(&want));
    assert_eq!(
        (rail.anchor, rail.width, rail.y.clone()),
        (
            PopupAnchor {
                edge: PopupEdge::Left,
                offset: 0
            },
            114,
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
<Component Name="MenuBar"><Images State="Normal"><Image Type="Middle">bar-110x60</Image></Images></Component>
<Component Name="XfireMenuText"><Images State="Normal"><Image Type="Middle">text-80x16</Image></Images></Component>
<Component Name="FileMenu"><Images State="Normal"><Image Type="Middle">menu-80x20</Image></Images></Component>
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

/// Plan SK2 lead decision 15 (XGlow's `Gripper`, `Xglow.ini:229-233`, 34×30
/// art 14 px past the window's right and 10 px past its bottom; the original
/// shows none of it): a fixed tile and its state images are clipped to its
/// parent, the served art is the visible part, the node keeps its role, and
/// the window no longer grows to hold what was clipped.
#[test]
fn a_fixed_tile_is_clipped_to_its_parent() {
    let main = parse(
        "main.xml",
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="Top" X="0" Y="0" ResizeX="100" Component="Top"/>
<Tile Name="Gripper" JustX="RIGHT" X="-14" JustY="BOTTOM" Y="-10" Component="Grip"><Triggers><Trigger Type="OnLeftButtonDown">ResizeBR</Trigger></Triggers></Tile>
<Tile Name="CloseBox" JustX="RIGHT" X="-6" Y="2" Component="Close"/>
</Tile>"#,
    )
    .unwrap();
    let parts = parse(
        "parts.xml",
        br#"<Components>
<Component Name="Top"><Images State="Normal"><Image Type="Middle">top-10x20</Image></Images></Component>
<Component Name="Grip"><Images State="Normal"><Image Type="Middle">grip-34x30</Image></Images></Component>
<Component Name="Close"><Images State="Normal"><Image Type="Middle">close-16x12</Image></Images>
<Images State="Hover"><Image Type="Middle">closeh-16x12</Image></Images></Component>
</Components>"#,
    )
    .unwrap();
    let mut built = (frame(), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &[],
        &mut built,
    )
    .unwrap();
    let tree = built.0.tree.clone().unwrap();
    assert_eq!(tree.outsets, Z);
    let by = |id: &str| tree.root.iter().find(|n| n.id == id).unwrap();
    let end = |size| NodeSpan::Fixed {
        at: Anchor::End(0),
        size,
    };
    assert_eq!(
        (by("Gripper").x.clone(), by("Gripper").y.clone()),
        (end(20), end(20))
    );
    let close = by("CloseBox");
    assert_eq!(close.x, end(10));
    let art = close.art.as_ref().unwrap();
    let width = |p: &str| {
        let png = &built.1[p];
        u32::from_be_bytes(png[16..20].try_into().unwrap())
    };
    let image = |set: &rekindle_types::dto::skin_tree::NodePieces| {
        set.values().next().unwrap().image.clone()
    };
    assert_eq!(width(&image(&art.normal)), 10);
    assert_eq!(
        art.states
            .values()
            .map(|s| width(&image(s)))
            .collect::<Vec<_>>(),
        [10]
    );
}

/// Plan SK2 lead decision 32: Total Chaos's `BottomCenter` tab (171 px,
/// `JustX="CENTER"`) is centred in the uncut 1 + w px `BottomBar`, as Xfire
/// floors it: ⌊(w + 1 − 171) / 2⌋ − 1, x 149 in the original's 470 px
/// window (`skinref/total-chaos/buddy-list-1.png`), at odd and even widths.
#[test]
fn a_centred_child_of_a_cut_row_is_placed_in_the_uncut_row() {
    let (_, tree, _) = imported_art(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" OutdentLeft="1" Y="0" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="Content" X="0" OutdentLeft="1" Y="MainTitleBar.BOTTOM" IndentBottom="61" ResizeX="100" ResizeY="100" Component="Content"/>
<Tile Name="BottomBar" X="0" OutdentLeft="1" Y="Content.BOTTOM" ResizeX="100" Component="BottomBar">
<Tile Name="BottomCenter" JustX="CENTER" X="0" Y="0" Component="BottomCenter"/>
</Tile>
</Tile>"#,
        br#"<Components>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="Middle">bar-10x52</Image></Images></Component>
<Component Name="Content"><Images State="Normal"><Image Type="Middle">ct-10x10</Image></Images></Component>
<Component Name="BottomCenter"><Images State="Normal"><Image Type="Middle">bc-171x20</Image></Images></Component>
<Component Name="BottomBar"><Images State="Normal"><Image Type="left">bl-161x66</Image><Image Type="middle">bm-37x66</Image><Image Type="right">br-161x66</Image></Images></Component>
</Components>"#,
        &["MainTitleBar", "Content", "BottomBar"],
    );
    let at = |w: i32| {
        tree.boxes(w, 555)
            .iter()
            .find(|(n, _)| n.id == "BottomCenter")
            .unwrap()
            .1[0]
    };
    assert_eq!(at(470), 149);
    for w in [470, 471, 500, 501] {
        assert_eq!(at(w), (w + 1 - 171).div_euclid(2) - 1, "w = {w}");
    }
}
