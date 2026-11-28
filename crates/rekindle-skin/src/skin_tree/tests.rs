//! The Host's ten rules on a built tree, and the fixtures the other tree tests share (plan SK2-S1).

use std::collections::BTreeMap;

use rekindle_image::Rgba;
use rekindle_types::dto::skin::{Insets, WindowLayout};
use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{
    Anchor, NodeArt, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey,
};

use super::{css, validate};

pub(super) const ON: Rgba<u8> = Rgba([10, 20, 30, 255]);

pub(super) fn layout(tree: Option<SkinTree>) -> WindowLayout {
    let z = Insets {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };
    WindowLayout {
        outsets: z,
        content: z,
        min_width: 200,
        min_height: 300,
        tiles: Vec::new(),
        title: None,
        tree,
        popups: std::collections::BTreeMap::new(),
    }
}

pub(super) fn fixed(at: i16, size: u16) -> NodeSpan {
    NodeSpan::Fixed {
        at: Anchor::Start(at),
        size,
    }
}

/// `node` run by `on` → `run` (a control's trigger, design §2.4).
pub(super) fn acts(mut node: SkinNode, on: SkinEvent, run: SkinAction) -> SkinNode {
    node.triggers.push(NodeTrigger { on, run });
    node
}

pub(super) fn n(id: &str, role: NodeRole, x: NodeSpan, y: NodeSpan) -> SkinNode {
    SkinNode {
        id: id.to_owned(),
        role,
        x,
        y,
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
        triggers: Vec::new(),
        children: Vec::new(),
    }
}

pub(super) fn whole() -> NodeSpan {
    NodeSpan::Stretch { start: 10, end: 10 }
}

/// A tree every rule passes: views with the friends view, its button row,
/// a close box and an add button.
pub(super) fn good() -> Vec<SkinNode> {
    let mut views = n(
        "Views",
        NodeRole::Views,
        whole(),
        NodeSpan::Stretch { start: 40, end: 60 },
    );
    views.children.push(n(
        "BuddyView",
        NodeRole::View(ViewKey::Friends),
        whole(),
        whole(),
    ));
    let mut bar = n(
        "Bar",
        NodeRole::ButtonBar,
        whole(),
        NodeSpan::Fixed {
            at: Anchor::End(10),
            size: 40,
        },
    );
    let mut row = n(
        "FriendsButtons",
        NodeRole::ButtonRow(ViewKey::Friends),
        whole(),
        whole(),
    );
    row.children.push(acts(
        n("AddButton", NodeRole::Button, fixed(4, 30), fixed(2, 16)),
        SkinEvent::Click,
        SkinAction::AddFriend,
    ));
    bar.children.push(row);
    vec![
        acts(
            n(
                "CloseBox",
                NodeRole::Button,
                NodeSpan::Fixed {
                    at: Anchor::End(4),
                    size: 18,
                },
                fixed(4, 16),
            ),
            SkinEvent::Click,
            SkinAction::Close,
        ),
        views,
        bar,
    ]
}

pub(super) fn checked(
    root: Vec<SkinNode>,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let tree = SkinTree {
        outsets: rekindle_types::dto::skin::Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root,
    };
    validate::check("t", &tree, &layout(Some(tree.clone())), images).map_err(|e| format!("{e:?}"))
}

pub(super) fn refused(root: Vec<SkinNode>, rule: &str) {
    let e = checked(root, &BTreeMap::new()).unwrap_err();
    assert!(e.contains(rule), "{e}");
}

#[test]
fn a_good_tree_passes() {
    assert_eq!(checked(good(), &BTreeMap::new()), Ok(()));
}

#[test]
fn rule_1_a_part_is_never_a_root() {
    let mut t = good();
    t.push(n("Piece", NodeRole::Part, fixed(0, 4), fixed(0, 4)));
    refused(t, "rule 1");
}

#[test]
fn rule_2_ids_are_unique() {
    let mut t = good();
    t.push(n("CloseBox", NodeRole::Decor, fixed(0, 4), fixed(0, 4)));
    refused(t, "rule 2");
}

#[test]
fn rule_3_nodes_lie_inside_the_window() {
    let mut t = good();
    t.push(n("Vent", NodeRole::Decor, fixed(190, 20), fixed(0, 4)));
    refused(t, "rule 3");
}

#[test]
fn rule_5_an_interactive_node_has_a_hit_area_in_the_window() {
    let mut t = good();
    t.push(acts(
        n("Min", NodeRole::Button, fixed(0, 0), fixed(30, 0)),
        SkinEvent::Click,
        SkinAction::Minimize,
    ));
    refused(t, "rule 5");
    // A window too small for any 24 px target refuses every interactive node.
    let mut t = vec![acts(
        n("Min", NodeRole::Button, fixed(0, 16), fixed(0, 16)),
        SkinEvent::Click,
        SkinAction::Minimize,
    )];
    let mut small = layout(None);
    small.min_width = 20;
    small.tree = Some(SkinTree {
        outsets: rekindle_types::dto::skin::Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: t.clone(),
    });
    let e = validate::check(
        "main",
        &SkinTree {
            outsets: rekindle_types::dto::skin::Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
            root: t.clone(),
        },
        &small,
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(format!("{e:?}").contains("rule 5"), "{e:?}");
    // Lead decision 2: 16 px tall at the top edge, its 24 px hit area is
    // shifted inward (down) to stay in the window, not refused.
    t = good();
    t.push(acts(
        n("Min", NodeRole::Button, fixed(60, 16), fixed(0, 16)),
        SkinEvent::Click,
        SkinAction::Minimize,
    ));
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
    assert_eq!(
        validate::hit_area([60, 0, 16, 16], (200, 300)),
        [56, 0, 24, 24]
    );
    assert_eq!(
        validate::hit_area([190, 290, 10, 10], (200, 300)),
        [176, 276, 24, 24]
    );
}

/// Lead decision 2: a menu drawn as text has a 0 px import box; it is sized
/// by its text at render (`max-content`), so rule 5 does not refuse it, while
/// a 0 px button still is.
#[test]
fn rule_5_a_text_sized_menu_is_sized_at_render() {
    let mut t = good();
    t.push(acts(
        n(
            "XfireMenuText",
            NodeRole::MenuTitle,
            fixed(40, 0),
            fixed(30, 0),
        ),
        SkinEvent::LeftDown,
        SkinAction::Menu(EngineMenu::File),
    ));
    assert_eq!(checked(t.clone(), &BTreeMap::new()), Ok(()));
    let out = css::tree_css(
        ".app-frame",
        &SkinTree {
            outsets: rekindle_types::dto::skin::Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
            root: t,
        },
        &layout(None),
        &BTreeMap::new(),
        "x",
    )
    .unwrap();
    assert!(out.contains("[data-skin-node=\"XfireMenuText\"] {\n    position: absolute; left: 40px; width: max-content; top: 30px; height: max-content;"), "{out}");
    let mut t = good();
    t.push(acts(
        n("Chat", NodeRole::Button, fixed(40, 0), fixed(30, 0)),
        SkinEvent::Click,
        SkinAction::ChatFriend,
    ));
    refused(t, "rule 5");
}

/// Lead decision 7: only singleton roles are unique; a button bar, a
/// button or decor may repeat (Symbiosis's two button bars).
#[test]
fn rule_6_singleton_roles_are_unique() {
    let mut t = good();
    t.push(n("Drag1", NodeRole::Drag, fixed(0, 24), fixed(40, 24)));
    t.push(n("Drag2", NodeRole::Drag, fixed(40, 24), fixed(40, 24)));
    refused(t, "rule 6");
    let mut t = good();
    t.push(n("Views2", NodeRole::Views, fixed(40, 24), fixed(80, 24)));
    refused(t, "rule 6");
    let mut t = good();
    t.push(n("Bolt", NodeRole::Decor, fixed(0, 4), fixed(0, 4)));
    t.push(n("Bolt2", NodeRole::Decor, fixed(0, 4), fixed(0, 4)));
    t.push(n("Bar2", NodeRole::ButtonBar, whole(), fixed(250, 20)));
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
}

#[test]
fn rule_7_the_views_hold_the_friends_view() {
    let mut t = good();
    t[1].children[0].role = NodeRole::View(ViewKey::Servers);
    t[2].children[0].role = NodeRole::ButtonRow(ViewKey::Servers);
    refused(t, "rule 7");
}

#[test]
fn rule_8_a_button_row_has_its_view() {
    let mut t = good();
    t[2].children[0].role = NodeRole::ButtonRow(ViewKey::Files);
    refused(t, "rule 8");
}

#[test]
fn rule_9_art_is_in_the_package() {
    let mut t = good();
    t[0].art = Some(NodeArt::middle("images/node-closebox.png".to_owned(), 1, 1));
    refused(t.clone(), "rule 9");
    let images = BTreeMap::from([("images/node-closebox.png".to_owned(), vec![0u8])]);
    assert_eq!(checked(t, &images), Ok(()));
}

#[test]
fn a_tree_never_requires_a_component_the_pack_lacks() {
    // XGlow lays no close box and StealthX no name field: a tree with
    // neither, and no views at all, passes.
    let t = vec![acts(
        n(
            "MinimizeButton",
            NodeRole::Button,
            fixed(6, 24),
            fixed(13, 24),
        ),
        SkinEvent::Click,
        SkinAction::Minimize,
    )];
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
}
