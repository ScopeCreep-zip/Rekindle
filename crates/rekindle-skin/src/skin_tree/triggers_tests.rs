//! A pack's triggers read into the tree (plan SK2-A2, lead decision 36):
//! tile then component, typed, an unknown name refused, `GroupMessage(n)`
//! resolved to its group, the INI era's `Action.<event>=` keys alike.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_action::{
    AppSignal, Capability, NodeState, NodeTrigger, SkinAction, SkinEvent,
};
use rekindle_types::dto::skin_tree::{SkinNode, SkinTree};

use super::import;
use super::test_support::{frame, piece};
use crate::import::tree::parse;
use crate::import::ImportError;
use crate::{builtin, manifest, package};

const MAIN: &[u8] = br#"<Tile Name="W" ResizeX="100" ResizeY="100" Component="MainWindow">
<Tile Name="MainTitleBar" X="0" Y="0" ResizeX="100" Component="MainTitleBar">
<Tile Name="CloseBox" X="0" Y="0" Component="CloseBox"><Triggers><Trigger Type="OnDoubleClick">AppMaximize</Trigger></Triggers></Tile>
<Tile Name="TabArt" X="20" Y="0" Component="TabArt"/>
</Tile></Tile>"#;

fn parts(close: &str, tab: &str) -> String {
    format!(
        r#"<Components>
<Component Name="MainWindow" MinWidth="200" MinHeight="300"/>
<Component Name="MainTitleBar"><Images State="Normal"><Image Type="Middle">bar-200x40</Image></Images></Component>
<Component Name="CloseBox"><Triggers>{close}</Triggers><Images State="Normal"><Image Type="Middle">close-18x16</Image></Images></Component>
<Component Name="TabArt" GroupId="4"><Triggers>{tab}</Triggers><Images State="Normal"><Image Type="Middle">tab-30x16</Image></Images></Component>
</Components>"#
    )
}

fn tree_from(close: &str, tab: &str) -> Result<SkinTree, ImportError> {
    let main = parse("main.xml", MAIN).unwrap();
    let parts = parse("parts.xml", parts(close, tab).as_bytes()).unwrap();
    let mut built = (frame(), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &[],
        &mut built,
    )?;
    Ok(built.0.tree.unwrap())
}

fn find<'a>(nodes: &'a [SkinNode], id: &str) -> Option<&'a SkinNode> {
    nodes
        .iter()
        .find_map(|n| (n.id == id).then_some(n).or_else(|| find(&n.children, id)))
}

const ON_CLICK_MINIMIZE: &str = r#"<Trigger Type="OnClick">AppMinimize</Trigger>"#;

/// A pack trigger reaches its node typed, the tile's before the
/// component's, and survives the manifest's JSON.
#[test]
fn a_pack_trigger_round_trips_into_the_node() {
    let tree = tree_from(ON_CLICK_MINIMIZE, "").unwrap();
    let close = find(&tree.root, "CloseBox").unwrap();
    assert_eq!(
        close.triggers,
        [
            NodeTrigger {
                on: SkinEvent::DoubleClick,
                run: SkinAction::Maximize,
            },
            NodeTrigger {
                on: SkinEvent::Click,
                run: SkinAction::Minimize,
            },
        ]
    );
    let json = serde_json::to_string(&tree).unwrap();
    let back: SkinTree = serde_json::from_str(&json).unwrap();
    assert_eq!(back, tree);
    let shell = tree.shell(SkinWindow::Main);
    let shelled = &shell.root[0].children[0];
    assert_eq!(shelled.triggers, close.triggers);
}

/// An action or event outside the engine's vocabulary refuses the pack,
/// naming it.
#[test]
fn an_unknown_action_or_event_is_refused_naming_it() {
    for (trigger, name) in [
        (
            r#"<Trigger Type="OnClick">AppFlyAway</Trigger>"#,
            "AppFlyAway",
        ),
        (
            r#"<Trigger Type="OnWobble">AppMinimize</Trigger>"#,
            "OnWobble",
        ),
        (
            r#"<Trigger Type="OnClick">State(Sideways)</Trigger>"#,
            "State(Sideways)",
        ),
    ] {
        match tree_from(trigger, "") {
            Err(ImportError::UnknownAction { tile, name: n, .. }) => {
                assert_eq!((tile.as_str(), n.as_str()), ("CloseBox", name));
            }
            other => panic!("{trigger}: {other:?}"),
        }
    }
}

/// `GroupMessage(n)` selects page `n` of the component's own group
/// (`GroupId="4"`); app signals drive its state set.
#[test]
fn a_group_message_resolves_to_its_group_page() {
    let tab = r#"<Trigger Type="OnClick">GroupMessage(1)</Trigger><Trigger Type="OnAppCannotJoin">State(Disabled)</Trigger>"#;
    let tree = tree_from("", tab).unwrap();
    let node = find(&tree.root, "TabArt").unwrap();
    assert_eq!(
        node.triggers,
        [
            NodeTrigger {
                on: SkinEvent::Click,
                run: SkinAction::GroupPage { group: 4, page: 1 },
            },
            NodeTrigger {
                on: SkinEvent::App(AppSignal::Cannot(Capability::Join)),
                run: SkinAction::SetState(NodeState::Disabled),
            },
        ]
    );
    // A group no component declares is refused.
    let stray = r#"<Trigger Type="OnClick">GroupMessage(0,77)</Trigger>"#;
    assert!(tree_from("", stray).is_err());
}

/// An INI pack's `Action.click=` and `<state>.Trigger=` keys import as
/// the same triggers (XGlow-style sections read as tiles).
#[test]
fn an_ini_packs_action_click_imports() {
    let skin = crate::import::xfire_ini::ini::parse(
        "x.ini",
        b"[MainWindow]\nMinWidth=200\nMinHeight=300\nWindow1.Name=CloseBox\nWindow1.X=0\nWindow1.Y=0\n\
[CloseBox]\nGraphics.middle=close-18x16\nAction.click=AppMinimize\nAction.lbtndown=AppDragWindow\n\
Disabled.Trigger=AppCannotChat\n",
    )
    .unwrap();
    let (layout, components) = crate::import::xfire_ini::title::tree(&skin).unwrap();
    let mut built = (frame(), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &layout,
        std::slice::from_ref(&components),
        |path, _| Ok(piece(path)),
        &[],
        &mut built,
    )
    .unwrap();
    let tree = built.0.tree.unwrap();
    let close = find(&tree.root, "CloseBox").unwrap();
    assert_eq!(
        close.triggers,
        [
            NodeTrigger {
                on: SkinEvent::Click,
                run: SkinAction::Minimize,
            },
            NodeTrigger {
                on: SkinEvent::LeftDown,
                run: SkinAction::DragWindow,
            },
            NodeTrigger {
                on: SkinEvent::App(AppSignal::Cannot(Capability::Chat)),
                run: SkinAction::SetState(NodeState::Disabled),
            },
        ]
    );
}

/// Spec A11: Classic's `CloseBox` minimizes (`xfirecomponents.xml`
/// `<Trigger Type="OnClick">AppMinimize</Trigger>`), as the built-in
/// package carries it.
#[test]
fn classics_close_box_carries_app_minimize() {
    let (_, b) = builtin::find("builtin:xfire-classic").unwrap();
    let package = package::read(b.bytes).unwrap();
    let m = manifest::parse(&package).unwrap();
    let tree = m.windows[&SkinWindow::Main].tree.as_ref().unwrap();
    let close = find(&tree.root, "CloseBox").unwrap();
    assert!(
        close
            .triggers
            .iter()
            .any(|t| t.on == SkinEvent::Click && t.run == SkinAction::Minimize),
        "{:?}",
        close.triggers
    );
}
