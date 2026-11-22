//! Plan SK2 lead decision 6: overlapping interactive nodes stack by the
//! pack's document order, and an edge grip's hit area yields (rule 4);
//! decision 10: it yields by the window's global order; decision 9: a single
//! views control is one view per tab.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Anchor, Edge, NodeRole, NodeSpan, SkinTree, ViewKey};

use super::roles::{role, Facts, Role};
use super::tests::{acts, checked, fixed, good, layout, n, refused, whole};
use super::{css, passes, validate};

/// Lead decision 6: interactive nodes may overlap, stacked by `Z` then the
/// pack's document order; a node another wholly covers is unreachable.
#[test]
fn rule_4_overlap_stacks_by_document_order() {
    let min = |at: i16, size: u16| {
        acts(
            n(
                "Min",
                NodeRole::Button,
                NodeSpan::Fixed {
                    at: Anchor::End(at),
                    size,
                },
                fixed(4, 16),
            ),
            SkinEvent::Click,
            SkinAction::Minimize,
        )
    };
    // Overlapping the close box in part: allowed, the later one on top.
    let mut t = good();
    t.push(min(10, 18));
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
    // Laid over the whole close box, later in document order: unreachable.
    let mut t = good();
    t.push(min(1, 24));
    refused(t.clone(), "rule 4");
    // The close box raised above the wider minimize box by its Z: both
    // reachable again.
    t[0].z = 3;
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
    // Earlier in document order, the minimize box is the one covered.
    let mut t = good();
    t.insert(0, min(4, 18));
    let e = checked(t, &BTreeMap::new()).unwrap_err();
    assert!(e.contains("t.Min") && e.contains("rule 4"), "{e}");
}

/// Lead decision 6: an edge grip's hit area yields to an interactive node
/// it overlaps, never past the grip's own box.
#[test]
fn rule_4_an_edge_grips_hit_area_yields() {
    // A 5 px left-edge strip and a button 10 px in: the 24 px area stops at
    // the button.
    assert_eq!(
        validate::grip_area([0, 0, 5, 300], (200, 300), &[[10, 100, 30, 16]]),
        [0, 0, 10, 300]
    );
    // A button over the strip itself (lead decision 10): the strip is cut
    // short of it along its length, the larger piece kept, and grown.
    assert_eq!(
        validate::grip_area([0, 0, 5, 300], (200, 300), &[[2, 100, 30, 16]]),
        [0, 116, 24, 184]
    );
    // A bottom strip: trimmed upward-facing, toward the bottom edge.
    assert_eq!(
        validate::grip_area([0, 295, 200, 5], (200, 300), &[[50, 270, 30, 20]]),
        [0, 290, 200, 10]
    );
    // Nothing near: the full 24 px area.
    assert_eq!(
        validate::grip_area([0, 0, 5, 300], (200, 300), &[]),
        validate::hit_area([0, 0, 5, 300], (200, 300))
    );
    let mut t = good();
    t.push(acts(
        n("ResizeLeft", NodeRole::Button, fixed(0, 5), whole()),
        SkinEvent::LeftDown,
        SkinAction::Resize(Edge::Left),
    ));
    t.push(acts(
        n("Chat", NodeRole::Button, fixed(10, 30), fixed(100, 16)),
        SkinEvent::Click,
        SkinAction::ChatFriend,
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
    assert!(
        out.contains("[data-skin-node=\"ResizeLeft\"]::before {\n    content: \"\";\n    position: absolute;\n    left: 0px;\n    top: 0px;\n    width: 10px;"),
        "{out}"
    );
}

fn frame_tree(root: Vec<rekindle_types::dto::skin_tree::SkinNode>) -> SkinTree {
    SkinTree {
        outsets: rekindle_types::dto::skin::Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root,
    }
}

/// Lead decision 10 (xSLIMFIRE): a root `ResizeUp` `Z=2` over a `CloseBox`
/// `Z=3` inside a `Z=1` title strip yields to it: the close box is above
/// the grip by effective z, so rule 4 holds and the grip's hit area (its
/// own box takes no pointer events) stops short of the close box.
#[test]
fn rule_4_a_grip_yields_by_effective_z() {
    let mut t = good();
    let mut close = t.remove(0);
    close.z = 3;
    let mut title = n("MainTitleBar", NodeRole::Drag, whole(), fixed(0, 24));
    title.z = 1;
    title.children.push(close);
    t.insert(0, title);
    let mut grip = acts(
        n("ResizeUp", NodeRole::Button, whole(), fixed(0, 24)),
        SkinEvent::LeftDown,
        SkinAction::Resize(Edge::Top),
    );
    grip.z = 2;
    t.push(grip);
    assert_eq!(checked(t.clone(), &BTreeMap::new()), Ok(()));
    let tree = frame_tree(t.clone());
    let out = css::tree_css(".app-frame", &tree, &layout(None), &BTreeMap::new(), "x").unwrap();
    let w = i32::from(layout(None).min_width);
    let boxes = tree.boxes(w, 300);
    let at = |id: &str| boxes.iter().find(|(n, _)| n.id == id).map(|(_, b)| *b);
    let (grip, close) = (at("ResizeUp").unwrap(), at("CloseBox").unwrap());
    let want = format!(
        "[data-skin-node=\"ResizeUp\"]::before {{\n    content: \"\";\n    position: absolute;\n    left: 0px;\n    top: 0px;\n    width: {}px;",
        close[0] - grip[0]
    );
    assert!(out.contains(&want), "{out}");
    assert!(out.contains("z-index: 3; pointer-events: none;"), "{out}");
    // The close box lowered below the grip: the grip wholly covers it.
    if let Some(c) = t[0].children.first_mut() {
        c.z = 0;
    }
    t[0].z = 0;
    refused(t, "rule 4");
}

/// Lead decision 9 (Symbiosis): the single `XfireView` control is the
/// views, split into one view per tab in the tabs' order.
#[test]
fn a_single_views_control_is_one_view_per_tab() {
    let facts = Facts {
        has_art: false,
        class: Some("XfireView"),
        children: &[],
    };
    let views = role("XfireView", Some("XfireView"), None, &facts);
    assert_eq!(views, Some(Role::Shows(NodeRole::Views)));
    let mut strip = n("Tabs", NodeRole::TabStrip, whole(), fixed(0, 20));
    for (id, v) in [
        ("ButtonFriendsTab", ViewKey::Friends),
        ("ButtonChatRoomsTab", ViewKey::ChatRooms),
        ("ButtonFriendsTab2", ViewKey::Friends),
    ] {
        strip
            .children
            .push(n(id, NodeRole::Tab(v), fixed(0, 30), fixed(0, 20)));
    }
    let mut root = vec![strip, n("XfireView", NodeRole::Views, whole(), whole())];
    passes::tab_views(&mut root);
    let made: Vec<_> = root[1]
        .children
        .iter()
        .map(|c| (c.id.as_str(), c.role, c.shows_with))
        .collect();
    assert_eq!(
        made,
        [
            (
                "XfireView.Friends",
                NodeRole::View(ViewKey::Friends),
                Some(ViewKey::Friends)
            ),
            (
                "XfireView.ChatRooms",
                NodeRole::View(ViewKey::ChatRooms),
                Some(ViewKey::ChatRooms)
            ),
        ]
    );
}

/// A button bar is a tab control showing one child at a time: StealthX's
/// `ChatTab` lays `HostChatButton`, `JoinChatButton` and `LeaveChatButton`
/// in one place and shows one by chat state (`GroupMessage(0/1/2)`,
/// `components.xml:705-712`), so none covers another. The same buttons in
/// a plain group stay refused.
#[test]
fn rule_4_a_button_bars_pages_are_never_shown_together() {
    let bar = |role| {
        let mut b = n("ChatTab", role, fixed(20, 110), fixed(60, 21));
        b.children = [
            ("HostChatButton", SkinAction::HostVoiceChat),
            ("JoinChatButton", SkinAction::VoiceChat),
            ("LeaveChatButton", SkinAction::LeaveVoiceChat),
        ]
        .into_iter()
        .map(|(id, a)| {
            acts(
                n(id, NodeRole::Button, fixed(0, 110), fixed(0, 21)),
                SkinEvent::Click,
                a,
            )
        })
        .collect();
        b
    };
    let mut t = good();
    t.push(bar(NodeRole::ButtonBar));
    assert_eq!(checked(t, &BTreeMap::new()), Ok(()));
    let mut t = good();
    t.push(bar(NodeRole::Group));
    refused(t, "rule 4");
}
