//! The tree importer's anchors and the Host's ten rules (plan SK2-S1).

use std::collections::BTreeMap;

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{
    ActionKey, Anchor, MenuKey, NodeArt, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey,
};

use super::roles::{role, Facts};
use super::{css, import, validate};
use crate::import::tree::parse;

const ON: Rgba<u8> = Rgba([10, 20, 30, 255]);

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
    }
}

pub(super) fn fixed(at: i16, size: u16) -> NodeSpan {
    NodeSpan::Fixed {
        at: Anchor::Start(at),
        size,
    }
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
        shows_with: None,
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
    row.children.push(n(
        "AddButton",
        NodeRole::Button(ActionKey::AddFriend),
        fixed(4, 30),
        fixed(2, 16),
    ));
    bar.children.push(row);
    vec![
        n(
            "CloseBox",
            NodeRole::Close,
            NodeSpan::Fixed {
                at: Anchor::End(4),
                size: 18,
            },
            fixed(4, 16),
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
    t.push(n("Min", NodeRole::Minimize, fixed(0, 0), fixed(30, 0)));
    refused(t, "rule 5");
    // A window too small for any 24 px target refuses every interactive node.
    let mut t = vec![n("Min", NodeRole::Minimize, fixed(0, 16), fixed(0, 16))];
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
    t.push(n("Min", NodeRole::Minimize, fixed(60, 16), fixed(0, 16)));
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
    t.push(n(
        "XfireMenuText",
        NodeRole::Menu(MenuKey::First),
        fixed(40, 0),
        fixed(30, 0),
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
    t.push(n(
        "Chat",
        NodeRole::Button(ActionKey::Chat),
        fixed(40, 0),
        fixed(30, 0),
    ));
    refused(t, "rule 5");
}

/// Lead decision 7: only singleton roles are unique; a button bar, a
/// button or decor may repeat (Symbiosis's two button bars).
#[test]
fn rule_6_singleton_roles_are_unique() {
    let mut t = good();
    t.push(n("Close2", NodeRole::Close, fixed(40, 24), fixed(40, 24)));
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
    t[0].art = Some(NodeArt {
        image: "images/node-closebox.png".to_owned(),
        states: BTreeMap::new(),
    });
    refused(t.clone(), "rule 9");
    let images = BTreeMap::from([("images/node-closebox.png".to_owned(), vec![0u8])]);
    assert_eq!(checked(t, &images), Ok(()));
}

#[test]
fn a_tree_never_requires_a_component_the_pack_lacks() {
    // XGlow lays no close box and StealthX no name field: a tree with
    // neither, and no views at all, passes.
    let t = vec![n(
        "MinimizeButton",
        NodeRole::Minimize,
        fixed(6, 24),
        fixed(13, 24),
    )];
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
}

/// Classic v3's title tiles (`xfiremainwindow.xml:2-12`) and a views box.
const CLASSIC: &[u8] = br#"<Tile Name="XfireMainWindow" Component="MainWindow">
<Tile Name="MainTitleBar" Z="4" X="0" Y="0" IndentLeft="14" IndentRight="13+14" ResizeX="100" Component="MainTitleBar">
<Tile Name="XfireLogo" X="20" Y="20" Component="XfireLogo"/>
<Tile Name="UserNameField" X="XfireLogo.RIGHT+6" Y="XfireLogo.TOP+2" ResizeX="100" IndentRight="48" Component="UserNameField">
<Tile Name="UserNameText" X="13" Y="0" ResizeX="100" IndentRight="13" Component="UserNameText"/></Tile>
<Tile Name="CloseBox" X="UserNameField.RIGHT+1" Y="UserNameField.TOP" Component="CloseBox"/>
</Tile>
<Tile Name="TabbedControl" X="7" Y="MainTitleBar.BOTTOM" IndentTop="9" IndentBottom="20+3" IndentRight="6" ResizeX="100" ResizeY="100" Component="TabbedControl">
<Tile Name="BuddyView" ResizeX="100" ResizeY="100" Component="BuddyView"/></Tile>
</Tile>"#;

const PARTS: &[u8] = br#"<Components>
<Component Name="MainTitleBar"><Images State="Normal"><Image>bar-300x50</Image></Images></Component>
<Component Name="XfireLogo"><Images State="Normal"><Image>logo-40x30</Image></Images></Component>
<Component Name="CloseBox"><Images State="Normal"><Image>close-18x16</Image></Images></Component>
</Components>"#;

fn piece(path: &str) -> RgbaImage {
    let (w, h) = path
        .rsplit_once('-')
        .and_then(|(_, d)| d.split_once('x'))
        .unwrap();
    RgbaImage::from_pixel(w.parse().unwrap(), h.parse().unwrap(), ON)
}

fn imported(xml: &[u8]) -> Result<(WindowLayout, SkinTree), crate::import::ImportError> {
    imported_with(xml, PARTS)
}

fn imported_with(
    xml: &[u8],
    parts: &[u8],
) -> Result<(WindowLayout, SkinTree), crate::import::ImportError> {
    let main = parse("main.xml", xml).unwrap();
    let parts = parse("parts.xml", parts).unwrap();
    let mut built = (layout(None), BTreeMap::new());
    import::add(
        SkinWindow::Main,
        &main,
        std::slice::from_ref(&parts),
        |path, _| Ok(piece(path)),
        &mut built,
    )?;
    let tree = built.0.tree.clone().unwrap();
    Ok((built.0, tree))
}

#[test]
fn classic_anchors_resolve_parent_relative() {
    let (l, tree) = imported(CLASSIC).unwrap();
    let bar = &tree.root[0];
    assert_eq!(bar.role, NodeRole::Drag);
    assert_eq!(bar.x, NodeSpan::Stretch { start: 14, end: 27 });
    let [logo, name, close] = &bar.children[..] else {
        panic!("{:?}", bar.children)
    };
    assert_eq!((logo.role, logo.x), (NodeRole::Logo, fixed(20, 40)));
    // `X="XfireLogo.RIGHT+6"`: 20 + 40 + 6.
    assert_eq!(
        (name.role, name.x),
        (
            NodeRole::NameField,
            NodeSpan::Stretch { start: 66, end: 48 }
        )
    );
    assert_eq!(name.y, fixed(22, 0));
    assert_eq!(name.children[0].role, NodeRole::Part);
    // `X="UserNameField.RIGHT+1"`: the stretching sibling's right inset, less 1, less its width.
    assert_eq!(
        close.x,
        NodeSpan::Fixed {
            at: Anchor::End(29),
            size: 18
        }
    );
    assert!(close.art.is_some());
    // The views box is the content: 7 left, 50 + 9 top, 6 right, 23 bottom.
    let c = l.content;
    assert_eq!((c.left, c.top, c.right, c.bottom), (7, 59, 6, 23));
}

#[test]
fn rule_1_an_unknown_component_without_art_is_refused() {
    let xml = br#"<Tile Name="W" Component="MainWindow"><Tile Name="Mystery" Component="Mystery"/></Tile>"#;
    let e = imported(xml).unwrap_err();
    assert!(e.to_string().contains("has no role"), "{e}");
}

#[test]
fn rule_2_an_anchor_that_does_not_resolve_is_refused() {
    let xml = br#"<Tile Name="W" Component="MainWindow"><Tile Name="CloseBox" X="Nowhere.RIGHT+1" Component="CloseBox"/></Tile>"#;
    let e = imported(xml).unwrap_err();
    assert!(e.to_string().contains("no sibling Nowhere"), "{e}");
}

#[test]
fn css_lays_one_rule_per_node_with_a_hit_area() {
    let tree = SkinTree {
        outsets: rekindle_types::dto::skin::Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: good(),
    };
    let out = css::tree_css(".app-frame", &tree, &layout(None), &BTreeMap::new(), "x").unwrap();
    assert_eq!(
        out.matches(" {\n    position: absolute;").count(),
        6,
        "{out}"
    );
    assert!(out.contains("[data-skin-node=\"CloseBox\"] {\n    position: absolute; right: 4px; width: 18px; top: 4px; height: 16px; z-index: 0;"), "{out}");
    assert!(
        out.contains("[data-skin-node=\"CloseBox\"]::before"),
        "{out}"
    );
    assert!(!out.contains("[data-skin-node=\"Views\"]::before"), "{out}");
    // Lead decision 2: CloseBox (18x16 at right 4, top 4) grows to 24x24
    // shifted inward: 3 px left of centre would leave the top edge.
    assert!(out.contains("[data-skin-node=\"CloseBox\"]::before {\n    content: \"\";\n    position: absolute;\n    left: -3px;\n    top: -4px;\n    width: 24px;\n    height: 24px;"), "{out}");
    // Lead decision 3: the views node draws opaque over the window art.
    assert!(out.contains("[data-skin-node=\"Views\"] {\n    position: absolute; left: 10px; right: 10px; top: 40px; bottom: 60px; z-index: 0; background-color: var(--color-xfire-bg-dark);"), "{out}");
}

fn root_of(xml: &[u8]) -> Vec<SkinNode> {
    imported(xml).unwrap().1.root
}

/// Symbiosis's `ResizeTop` `IndentRight="ResizeGripperTR.Width+13"` names
/// the tile after it: tiles are placed in as many passes as they need.
#[test]
fn a_reference_to_a_later_sibling_resolves() {
    let kids = root_of(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" ResizeX="100" IndentRight="CloseBox.WIDTH+2" Component="MainTitleBar"/>
<Tile Name="CloseBox" JustX="RIGHT" X="4" Component="CloseBox"/></Tile>"#,
    );
    assert_eq!(kids[0].x, NodeSpan::Stretch { start: 0, end: 20 });
}

/// StealthX's `WndMax` `JustX="RIGHT" X="WndClose.LEFT-3"`: the sibling's
/// edge is a point, and the tile's far edge sits on it.
#[test]
fn a_right_justified_tile_ends_on_a_siblings_edge() {
    let kids = root_of(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustX="RIGHT" X="4" Component="CloseBox"/>
<Tile Name="XfireLogo" JustX="RIGHT" X="CloseBox.LEFT-3" Component="XfireLogo"/></Tile>"#,
    );
    let at = NodeSpan::Fixed {
        at: Anchor::End(25),
        size: 40,
    };
    assert_eq!(kids[1].x, at);
}

/// Combat's `JoinPlayTab` `X="CenterConsole.RIGHT"`: an edge of a centred
/// sibling places the tile from the centre too (at 200 px: 111 both ways).
#[test]
fn a_tile_beside_a_centred_sibling_is_centred() {
    let kids = root_of(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="CloseBox" JustX="CENTER" X="0" Component="CloseBox"/>
<Tile Name="XfireLogo" X="CloseBox.RIGHT+2" Component="XfireLogo"/></Tile>"#,
    );
    let at = NodeSpan::Fixed {
        at: Anchor::Centre(31),
        size: 40,
    };
    assert_eq!(kids[1].x, at);
}

/// Total Chaos's `AdArea` `Y="MainTitleBar.BOTTOM"` inside `AdBar`, itself
/// at `MainTitleBar.BOTTOM`: a name of the parent's siblings, in the
/// child's frame.
#[test]
fn a_child_sees_its_parents_siblings_in_its_own_frame() {
    let kids = root_of(
        br#"<Tile Name="W" Component="MainWindow">
<Tile Name="MainTitleBar" ResizeX="100" Component="MainTitleBar"/>
<Tile Name="AdBar" Y="MainTitleBar.BOTTOM" ResizeX="100" Component="AdBar">
<Tile Name="AdArea" Y="MainTitleBar.BOTTOM" JustX="CENTER" Component="AdArea"/></Tile></Tile>"#,
    );
    assert_eq!(kids[1].y, fixed(50, 0));
    assert_eq!(kids[1].children[0].y, fixed(0, 0));
    assert_eq!(kids[1].children[0].role, NodeRole::Part);
}

/// xSLIMFIRE's tabs stretch (`ResizeX="100"`) up to their component's
/// `MaxWidth="65"`, each 12 px after the last.
#[test]
fn a_stretch_is_capped_by_its_components_max_width() {
    let parts = br#"<Components>
<Component Name="TabNormalButton" Class="PushButton" MaxWidth="65" MaxHeight="19"/>
<Component Name="TabServerButton" Class="PushButton" MaxWidth="65" MaxHeight="19"/>
</Components>"#;
    let xml = br#"<Tile Name="TabHeader" X="23" ResizeX="100" IndentRight="23" Component="TabHeader">
<Tile Name="TabNormalButton" ResizeX="100" ResizeY="100" Component="TabNormalButton"/>
<Tile Name="TabServerButton" X="TabNormalButton.RIGHT+12" Y="1" ResizeX="100" ResizeY="100" Component="TabServerButton"/>
</Tile>"#;
    let tree = imported_with(xml, parts).unwrap().1;
    let [normal, server] = &tree.root[..] else {
        panic!("{:?}", tree.root)
    };
    assert_eq!(normal.role, NodeRole::Tab(ViewKey::Friends));
    assert_eq!((normal.x, normal.y), (fixed(0, 65), fixed(0, 19)));
    assert_eq!((server.x, server.y), (fixed(77, 65), fixed(1, 19)));
}

/// Simple X's `AdArea` (`SimpleX.ini:121-127`, `[AdArea]` `MaxWidth=240`):
/// a centred stretch under its component's cap is centred over the span it
/// would stretch across, never pinned to that span's start.
#[test]
fn a_centred_capped_stretch_is_centred_in_its_span() {
    let parts =
        br#"<Components><Component Name="AdArea" Class="Tile" MaxWidth="240"/></Components>"#;
    let xml = br#"<Tile Name="MainWindow" ResizeX="100" ResizeY="100">
<Tile Name="AdArea" JustX="CENTER" Y="29" ResizeX="100" OutdentLeft="7" IndentRight="7" Component="AdArea"/>
</Tile>"#;
    let tree = imported_with(xml, parts).unwrap().1;
    let [ad] = &tree.root[..] else {
        panic!("{:?}", tree.root)
    };
    let want = NodeSpan::Fixed {
        at: Anchor::Centre(-7),
        size: 240,
    };
    assert_eq!(ad.x, want);
}

/// A `Title` names the user only as text: Combat's `[Title]` is
/// `Class=Text` (`combat.ini:541-544`), Total Chaos's is the "T.C" plaque,
/// `Class="Tile"` art (`components.xml:451-460`).
#[test]
fn a_title_is_the_name_field_only_as_text() {
    let facts = |class, has_art| Facts {
        has_art,
        class: Some(class),
        children: &[],
    };
    let text = role("Title", Some("Title"), None, &facts("Text", false));
    assert_eq!(text, Some(NodeRole::NameField));
    let plaque = role("Title", Some("Title"), None, &facts("Tile", true));
    assert_eq!(plaque, Some(NodeRole::Decor));
}

/// SimpleX's `[TabAddButton]` `Class=TabControl` holds `AddButton`,
/// `AddServerButton` and `SearchFiles`: one slot of the button row, a
/// group of buttons; Combat's knob control is the tab strip.
#[test]
fn an_ini_tab_control_slot_is_a_group_of_buttons() {
    let kids = ["AddButton".to_owned(), "SearchFiles".to_owned()];
    let facts = Facts {
        has_art: false,
        class: Some("TabControl"),
        children: &kids,
    };
    let slot = role(
        "TabAddButton",
        Some("TabAddButton"),
        Some(NodeRole::Group),
        &facts,
    );
    assert_eq!(slot, Some(NodeRole::Group));
    let plain = Facts {
        has_art: true,
        class: Some("PushButton"),
        children: &[],
    };
    let search = role("SearchFiles", Some("SearchFiles"), slot, &plain);
    assert_eq!(search, Some(NodeRole::Button(ActionKey::Find)));
    let knobs = ["Knob01".to_owned(), "Knob04".to_owned()];
    let strip = Facts {
        has_art: false,
        class: Some("TabControl"),
        children: &knobs,
    };
    let tc = role(
        "TabControl",
        Some("TabControl"),
        Some(NodeRole::Group),
        &strip,
    );
    assert_eq!(tc, Some(NodeRole::TabStrip));
    let fourth = role("Knob04", Some("Knob04"), tc, &plain);
    assert_eq!(fourth, Some(NodeRole::Part));
}

/// StealthX's rail `InfoJoinChatTab` (`Component="ChatTab"`,
/// `Class="TabControl"`) holds one button per chat state (`popups.xml:75-81`):
/// a button bar of pages, not a per-view slot (its name reads "info"/"chat").
#[test]
fn a_tab_control_of_chat_state_buttons_is_a_button_bar() {
    let kids = [
        "HostChatButton".to_owned(),
        "JoinChatButton".to_owned(),
        "LeaveChatButton".to_owned(),
    ];
    let facts = Facts {
        has_art: false,
        class: Some("TabControl"),
        children: &kids,
    };
    let bar = role(
        "InfoJoinChatTab",
        Some("ChatTab"),
        Some(NodeRole::Decor),
        &facts,
    );
    assert_eq!(bar, Some(NodeRole::ButtonBar));
}
