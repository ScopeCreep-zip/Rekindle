//! Each built-in's chat-window tree (plan SK17-2, the INI packs' SK17-3):
//! the title strip
//! composed from the pack's own chat-window vocabulary, spot-checked
//! against the pack's chat layout file, the brand rule, and the slots the
//! shell mounts the conversation into, against their goldens.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, SlotKey, WindowButtonKind};

use super::golden_tests::lines;
use crate::{builtin, images, manifest, package};

/// The built-ins with a chat window: the four XML packs (Space Cowboy's
/// `ChatEdit`, inside `ListViewBG2`, anchors `Y="ChatArea.BOTTOM+2"` on
/// its parent's sibling, which [`super::anchor::outer_for`] resolves) and
/// the four INI packs whose `ChatSkin=` file lays one (plan SK17-3).
const PACKS: [&str; 8] = [
    "onyx",
    "xslimfire",
    "symbiosis",
    "spacecowboy",
    "combat",
    "alloy",
    "simple-x",
    "xglow",
];

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
        ("spacecowboy", include_str!("goldens/chat-spacecowboy.txt")),
        ("combat", include_str!("goldens/chat-combat.txt")),
        ("alloy", include_str!("goldens/chat-alloy.txt")),
        ("simple-x", include_str!("goldens/chat-simple-x.txt")),
        ("xglow", include_str!("goldens/chat-xglow.txt")),
    ];
    // Every pack is compared before failing, each differing tree in the
    // message whole, so one run regenerates them all.
    let mut wrong = Vec::new();
    for (id, golden) in want {
        let got = tree_text(chat_layout(id).tree.as_ref().unwrap());
        if got != golden {
            wrong.push(format!("=== chat-{id} begin\n{got}=== chat-{id} end"));
        }
    }
    assert!(
        wrong.is_empty(),
        "chat trees differ from their goldens:\n{}",
        wrong.join("\n")
    );
}

/// The title strip at the pack's own coordinates: Onyx's `MainTitleBar`
/// X5 Y5 with `Logo` X2 Y1 in it and `MenuBar` Y47 (`chatwindow.xml`),
/// Space Cowboy's `MenuBar` X30 Y64, xSLIMFIRE's `ChatTitleBar` at the
/// top, 165 tall, and `ChatMenuBar` X19 Y33, Symbiosis's `InfoBar` X5 Y5
/// (`ChatWindow.xml`); Combat's `MainTitleBar` `X=LeftEdge.CENTER X=60
/// OutdentLeft=1 Y=13` with `Logo` X-30 Y-3 in it and `GCMenuBar` `X=34
/// Y=Top.BOTTOM Y=-7` (`CombatChatSkin.ini:33-36,98-105,220-222`), Alloy's
/// `MainTitleBar` X0 Y0, `ChatLogo` X15 Y15, `MenuBar` X19 Y46
/// (`Chatskin.ini:14-53`), Simple X's X0 Y0, X9 Y5, X13 Y34 (`Chatskin.ini:
/// 18-59`); XGlow lays no title bar, logo or menu row (`ChatSkin.ini:
/// 14-132`: its `[Top]` is frame art). Each laid inside the window's
/// outsets.
#[test]
fn title_strips_sit_where_each_pack_places_them() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        let (w, h) = min_window(&l);
        let (ox, oy) = (i32::from(t.outsets.left), i32::from(t.outsets.top));
        assert_eq!(
            find(t, w, h, NodeRole::ChatTitle).len(),
            1,
            "{id}: one chat title"
        );
        if id == "xglow" {
            for role in [NodeRole::TitleBar, NodeRole::MenuBar, NodeRole::Logo] {
                assert_eq!(
                    find(t, w, h, role),
                    Vec::<[i32; 4]>::new(),
                    "{id}: {role:?}"
                );
            }
            let title = one(t, w, h, NodeRole::ChatTitle, id);
            assert_eq!((title[0] - ox, title[1] - oy), (4, 14), "{id} title");
            continue;
        }
        let bar = one(t, w, h, NodeRole::TitleBar, id);
        let menu = one(t, w, h, NodeRole::MenuBar, id);
        let logos = find(t, w, h, NodeRole::Logo);
        if id == "symbiosis" {
            // Symbiosis lays no chat logo: its `XfireLogo` tile is
            // commented out (`ChatWindow.xml:51`).
            assert_eq!(logos, Vec::<[i32; 4]>::new(), "{id}: a chat logo");
        }
        let logo = || one(t, w, h, NodeRole::Logo, id);
        match id {
            "onyx" => {
                assert_eq!((bar[0] - ox, bar[1] - oy), (5, 5), "{id} bar");
                assert_eq!(
                    (logo()[0] - bar[0], logo()[1] - bar[1]),
                    (2, 1),
                    "{id} logo"
                );
                assert_eq!((menu[0] - ox, menu[1] - oy), (7, 47), "{id} menu");
            }
            "spacecowboy" => {
                assert_eq!((menu[0] - ox, menu[1] - oy), (30, 64), "{id} menu");
                assert_eq!(
                    (logo()[0] - bar[0], logo()[1] - bar[1]),
                    (4, 3),
                    "{id} logo"
                );
            }
            "xslimfire" => {
                assert_eq!((bar[0] - ox, bar[1] - oy, bar[3]), (0, 0, 165), "{id} bar");
                assert_eq!(
                    (logo()[0] - bar[0], logo()[1] - bar[1]),
                    (20, 8),
                    "{id} logo"
                );
                assert_eq!((menu[0] - ox, menu[1] - oy), (19, 33), "{id} menu");
            }
            "combat" => {
                let named = |name: &str| {
                    t.boxes(w, h)
                        .into_iter()
                        .find(|(n, _)| n.id == name)
                        .map(|(_, b)| b)
                        .unwrap()
                };
                let (left, top) = (named("LeftEdge"), named("Top"));
                let bar_x = left[0] + left[2] / 2 + 60 - 1;
                assert_eq!((bar[0], bar[1] - oy), (bar_x, 13), "{id} bar");
                let at = (logo()[0] - bar[0], logo()[1] - bar[1]);
                assert_eq!(at, (-30, -3), "{id} logo");
                // Its whole 48×23 `c_chat_x_norm.gif`, never cut by the bar.
                assert_eq!((logo()[2], logo()[3]), (48, 23), "{id} logo size");
                let at = (menu[0] - ox, menu[1]);
                assert_eq!(at, (34, top[1] + top[3] - 7), "{id} menu");
            }
            "alloy" | "simple-x" => {
                let (logo_at, menu_at) = if id == "alloy" {
                    ((15, 15), (19, 46))
                } else {
                    ((9, 5), (13, 34))
                };
                assert_eq!((bar[0] - ox, bar[1] - oy), (0, 0), "{id} bar");
                assert_eq!((logo()[0] - ox, logo()[1] - oy), logo_at, "{id} logo");
                assert_eq!((menu[0] - ox, menu[1] - oy), menu_at, "{id} menu");
            }
            _ => {
                assert_eq!((bar[0] - ox, bar[1] - oy), (5, 5), "{id} bar");
            }
        }
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
            // xSLIMFIRE, Combat (`CombatChatSkin.ini:220-240`) and XGlow
            // (`ChatSkin.ini:14-132`) lay no maximize button.
            usize::from(!matches!(id, "xslimfire" | "combat" | "xglow")),
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
        // The pack's title node is the chat title: without its role the
        // pack's own words (Combat's `Xfire - %displayuser%`,
        // `CombatChatSkin.ini:243-249`) would be served in its place.
        let titles: Vec<_> = all
            .iter()
            .filter(|n| n.role == NodeRole::ChatTitle)
            .collect();
        assert_eq!(titles.len(), 1, "{id}: chat titles {titles:?}");
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

/// Combat's `[ContentArea]` (`CombatChatSkin.ini:291-297`): `ChatArea`
/// `Y=0 ResizeY=80` is 80% of the area's height (a share, never the fixed
/// height read at one size), at the served minimum and at 1200×900.
#[test]
fn combats_chat_area_is_80_percent_of_its_content_area() {
    let l = chat_layout("combat");
    let t = l.tree.as_ref().unwrap();
    for (w, h) in [min_window(&l), (1200, 900)] {
        let boxes = t.boxes(w, h);
        let area = boxes
            .iter()
            .find(|(n, _)| n.id.eq_ignore_ascii_case("ContentArea"))
            .map(|(_, b)| *b)
            .expect("combat: a ContentArea node");
        let chat = one(t, w, h, NodeRole::Slot(SlotKey::ChatArea), "combat");
        assert_eq!(
            chat[3],
            area[3] * 80 / 100,
            "combat at {w}x{h}: ChatArea {chat:?} in ContentArea {area:?}"
        );
    }
}
