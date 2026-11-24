//! Each XML built-in's chat-window tree (plan SK17-2): the title strip
//! composed from the pack's own chat-window vocabulary, spot-checked
//! against the pack's chat layout file, the brand rule, and the slots the
//! shell mounts the conversation into, against their goldens.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, SlotKey, WindowButtonKind};

use super::golden_tests::lines;
use crate::{builtin, images, manifest, package};

/// Space Cowboy waits: its `ChatEdit` (inside `ListViewBG2`) anchors
/// `Y="ChatArea.BOTTOM+2"` on its parent's sibling, which the anchor
/// resolver refuses (rule 2), so its chat window serves no tree yet.
const PACKS: [&str; 3] = ["onyx", "xslimfire", "symbiosis"];

/// The built-in's chat window layout, its package validated.
fn chat_layout(id: &str) -> WindowLayout {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let package = package::read(b.bytes).unwrap();
    let mut m = manifest::served(&package).unwrap();
    images::validate(&package, &m).unwrap();
    m.windows.remove(&SkinWindow::Chat).unwrap()
}

fn tree_text(tree: &SkinTree) -> String {
    let mut out = String::new();
    lines(&tree.root, 0, &mut out);
    out
}

/// The window box at the served minimum (outsets included).
fn min_window(l: &WindowLayout) -> (i32, i32) {
    super::target::window(l)
}

fn find(tree: &SkinTree, w: i32, h: i32, role: NodeRole) -> Vec<[i32; 4]> {
    tree.boxes(w, h)
        .into_iter()
        .filter(|(n, _)| n.role == role)
        .map(|(_, b)| b)
        .collect()
}

fn one(tree: &SkinTree, w: i32, h: i32, role: NodeRole, id: &str) -> [i32; 4] {
    let found = find(tree, w, h, role);
    assert_eq!(found.len(), 1, "{id}: {role:?} {found:?}");
    found[0]
}

#[test]
fn chat_trees_match_their_goldens() {
    let want = [
        ("onyx", include_str!("goldens/chat-onyx.txt")),
        ("xslimfire", include_str!("goldens/chat-xslimfire.txt")),
        ("symbiosis", include_str!("goldens/chat-symbiosis.txt")),
    ];
    for (id, golden) in want {
        let got = tree_text(chat_layout(id).tree.as_ref().unwrap());
        if got != golden {
            println!("=== chat-{id} begin\n{got}=== chat-{id} end");
        }
        assert_eq!(got, golden, "{id}: chat tree differs from its golden");
    }
}

/// The title strip at the pack's own coordinates: Onyx's `MainTitleBar`
/// X5 Y5 with `Logo` X2 Y1 in it and `MenuBar` Y47 (`chatwindow.xml`),
/// Space Cowboy's `MenuBar` X30 Y64, xSLIMFIRE's `ChatTitleBar` at the
/// top, 165 tall, and `ChatMenuBar` X19 Y33, Symbiosis's `InfoBar` X5 Y5
/// (`ChatWindow.xml`), each laid inside the window's outsets.
#[test]
fn title_strips_sit_where_each_pack_places_them() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        let (w, h) = min_window(&l);
        let (ox, oy) = (i32::from(t.outsets.left), i32::from(t.outsets.top));
        let bar = one(t, w, h, NodeRole::TitleBar, id);
        let menu = one(t, w, h, NodeRole::MenuBar, id);
        let logo = one(t, w, h, NodeRole::Logo, id);
        match id {
            "onyx" => {
                assert_eq!((bar[0] - ox, bar[1] - oy), (5, 5), "{id} bar");
                assert_eq!((logo[0] - bar[0], logo[1] - bar[1]), (2, 1), "{id} logo");
                assert_eq!((menu[0] - ox, menu[1] - oy), (7, 47), "{id} menu");
            }
            "spacecowboy" => {
                assert_eq!((menu[0] - ox, menu[1] - oy), (30, 64), "{id} menu");
                assert_eq!((logo[0] - bar[0], logo[1] - bar[1]), (4, 3), "{id} logo");
            }
            "xslimfire" => {
                assert_eq!((bar[0] - ox, bar[1] - oy, bar[3]), (0, 0, 165), "{id} bar");
                assert_eq!((logo[0] - bar[0], logo[1] - bar[1]), (20, 8), "{id} logo");
                assert_eq!((menu[0] - ox, menu[1] - oy), (19, 33), "{id} menu");
            }
            _ => {
                assert_eq!((bar[0] - ox, bar[1] - oy), (5, 5), "{id} bar");
                assert_eq!((logo[0] - bar[0], logo[1] - bar[1]), (65, 2), "{id} logo");
            }
        }
        assert_eq!(
            find(t, w, h, NodeRole::ChatTitle).len(),
            1,
            "{id}: one chat title"
        );
        let titles = find(t, w, h, NodeRole::MenuTitle).len();
        assert_eq!(titles, 3, "{id}: three menu titles");
    }
}

#[test]
fn window_buttons_are_the_packs_own() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        let (w, h) = min_window(&l);
        for kind in [WindowButtonKind::Minimize, WindowButtonKind::Close] {
            one(t, w, h, NodeRole::WindowButton(kind), id);
        }
        let max = find(t, w, h, NodeRole::WindowButton(WindowButtonKind::Maximize)).len();
        assert_eq!(
            max,
            usize::from(id != "xslimfire"),
            "{id}: maximize buttons"
        );
    }
    let l = chat_layout("symbiosis");
    let (w, h) = min_window(&l);
    one(
        l.tree.as_ref().unwrap(),
        w,
        h,
        NodeRole::TalkingState,
        "symbiosis",
    );
}

fn walk<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a SkinNode>) {
    for n in nodes {
        out.push(n);
        walk(&n.children, out);
    }
}

/// The logo is the Rekindle mark, no node shows Xfire's words, and the
/// chat title carries no words of the pack's (the shell writes the
/// conversation's title the Host serves the window).
#[test]
fn chat_trees_are_rekindle_branded_and_title_free() {
    for id in PACKS {
        let l = chat_layout(id);
        let mut all = Vec::new();
        walk(&l.tree.as_ref().unwrap().root, &mut all);
        for n in &all {
            if n.role == NodeRole::Logo {
                let label = n.label.as_ref();
                assert!(
                    label.is_some_and(|l| l.text == super::brand::BRAND && l.glyph.is_some()),
                    "{id}: logo {} without the Rekindle mark",
                    n.id
                );
            }
            if n.role == NodeRole::ChatTitle {
                assert!(n.label.is_none(), "{id}: {} serves the pack's words", n.id);
            }
            if let Some(w) = n.label.as_ref().map(|l| &l.text) {
                assert!(
                    !w.to_ascii_lowercase().contains("xfire"),
                    "{id}: {} shows {w:?}",
                    n.id
                );
            }
        }
    }
}

/// Every slot the pack draws has a box inside the window at the served
/// minimum and at 1200×900; every pack draws the message list, the
/// composer and the typing line.
#[test]
fn slots_sit_inside_the_window() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        for (w, h) in [min_window(&l), (1200, 900)] {
            let slots: Vec<_> = t
                .boxes(w, h)
                .into_iter()
                .filter_map(|(n, b)| match n.role {
                    NodeRole::Slot(k) => Some((k, b)),
                    _ => None,
                })
                .collect();
            for k in [SlotKey::ChatArea, SlotKey::ChatEdit, SlotKey::TypingNotice] {
                assert!(slots.iter().any(|(s, _)| *s == k), "{id}: no {k:?} slot");
            }
            for (k, [x, y, bw, bh]) in slots {
                assert!(
                    x >= 0 && y >= 0 && bw > 0 && bh > 0 && x + bw <= w && y + bh <= h,
                    "{id} at {w}x{h}: {k:?} {:?} outside",
                    [x, y, bw, bh]
                );
            }
        }
    }
}
