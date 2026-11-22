//! Plan SK2 design §2.3 (one layout model, superseding lead decision 15's
//! frame decoration): the side rail is a popup tree, and its nine-piece
//! ground is its own node's art in that tree, never the window's frame.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_tree::NodeRole;

use super::import;
use super::test_support::{frame, piece};
use crate::import::tree::parse;

/// Plan SK2 design §2.3: StealthX's rail exists only as a popup. Its
/// ground `MainMenuBackground` (`Component="MenuBackground"`, a nine-piece
/// set, `components.xml:26-40`) is that popup tree's node, carrying all
/// nine pieces as its art; the window lays no rail tile and no rail node.
#[test]
fn stealthxs_rail_exists_only_as_a_popup() {
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
<Component Name="MenuButton" Class="SkinLauncher"><Triggers><Trigger Type="OnClick">LaunchSkin</Trigger></Triggers><Images State="Normal"><Image Type="Middle">menu-20x60</Image></Images><Popup SaveState="1">MainMenuPopup</Popup></Component>
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
    assert!(
        built.0.tiles.iter().all(|t| !t.image.contains("rail")),
        "no rail frame tile"
    );
    let tree = built.0.tree.as_ref().unwrap();
    assert!(tree
        .boxes(0, 0)
        .iter()
        .all(|(n, _)| n.role != NodeRole::SideRail && n.id != "MainMenuBackground"));
    let rail = &built.0.popups["MainMenuPopup"];
    // The popup's box is the rail; its tree is the rail tile's children.
    assert_eq!(rail.tree.root.len(), 1);
    assert_eq!(rail.tree.root[0].id, "MainMenuBackground");
    let ground = rail
        .tree
        .boxes(0, 0)
        .into_iter()
        .find(|(n, _)| n.id == "MainMenuBackground")
        .map(|(n, _)| n)
        .unwrap();
    let art = ground.art.as_ref().expect("the ground's nine pieces");
    assert_eq!(art.normal.len(), 9);
    for p in art.normal.values() {
        assert!(built.1.contains_key(&p.image), "{} has its PNG", p.image);
    }
}
