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
        (
            "xfire-classic",
            include_str!("goldens/chat-xfire-classic.txt"),
        ),
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
            // The title is its `UserNameText`, `X=14` in the `UserNameField`
            // at X4 Y14 (`ChatSkin.ini:21-23,159-161`, plan SK17: the
            // pack's text point, [`super::chat_slots`]).
            assert_eq!((title[0] - ox, title[1] - oy), (18, 14), "{id} title");
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

/// The box of the node `id` in `tree` laid in `w` x `h`, and the far x
/// edge of its press area (its target box, else its box).
fn by_id(tree: &SkinTree, w: i32, h: i32, id: &str) -> ([i32; 4], i32) {
    tree.boxes(w, h)
        .into_iter()
        .find_map(|(n, b)| {
            let far = n
                .target
                .map_or(b[0] + b[2], |t| b[0] + t.area[0] + t.area[2]);
            (n.id == id).then_some((b, far))
        })
        .unwrap_or_else(|| panic!("no node {id}"))
}

/// Plan SK17: the chat title a pack lays at a point as `Class=Text
/// Stretch=ToContent` (the engine drew the name rightward from there) is
/// served the room its parent declares past that point, as StealthX's
/// `ResizeX=100` `UserNameText` is, never a 0 px box: Onyx's `Title` at
/// `X="Logo.RIGHT+2"` in the `ResizeX=100` `MainTitleBar`
/// (`chatwindow.xml:21-24`, `components.xml:1426-1430`), Symbiosis's
/// `ChatUser` at `X="TabbedTalkingState.Right+2"` in `ChatUserContainer`
/// (`ResizeX=100 IndentRight=93`, `ChatWindow.xml:56-62`,
/// `Components/ChatWindow.xml:131`), which nothing else in the container
/// meets past it. The name starts at the pack's point, or past the press
/// area of the node before it where that reaches further (plan SK13: the
/// 24 px target of Onyx's 16 px `Logo` ends 2 px past the point).
#[test]
fn a_point_laid_chat_title_spans_its_declared_room() {
    for (id, before, parent) in [
        ("onyx", "Logo", "MainTitleBar"),
        ("symbiosis", "TabbedTalkingState", "ChatUserContainer"),
    ] {
        let layout = chat_layout(id);
        let tree = layout.tree.as_ref().unwrap();
        let (width, height) = min_window(&layout);
        let title = one(tree, width, height, NodeRole::ChatTitle, id);
        let (prev, press) = by_id(tree, width, height, before);
        let (room, _) = by_id(tree, width, height, parent);
        assert!(title[2] > 0, "{id}: chat title {title:?} is 0 px wide");
        let start = (prev[0] + prev[2] + 2).max(press);
        assert_eq!(title[0], start, "{id}: title {title:?} after {prev:?}");
        let (end, room_end) = (title[0] + title[2], room[0] + room[2]);
        assert!(end <= room_end, "{id}: title {title:?} past {room:?}");
        if id == "symbiosis" {
            assert_eq!(end, room_end, "{id}: title {title:?} in {room:?}");
        } else {
            assert!(title[2] >= 100, "{id}: title {title:?} in {room:?}");
        }
    }
}

/// Plan SK17 (round 34: Alloy's title empty, xSLIMFIRE's drawn at the
/// window's corner): the chat title is the pack's text point, never the
/// field around it (Alloy's, Simple X's and XGlow's `UserNameText`,
/// xSLIMFIRE's `ChatUserNameText`, `xSLIMFIREchatwindow.xml:4-5`), with
/// room to draw, in the strip above the message list and off the logo.
#[test]
fn the_chat_title_is_the_packs_text_point_in_its_title_strip() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        let (w, h) = min_window(&l);
        let boxes = t.boxes(w, h);
        let (node, title) = boxes
            .iter()
            .find(|(n, _)| n.role == NodeRole::ChatTitle)
            .copied()
            .unwrap();
        let held: Vec<_> = node.children.iter().map(|c| &c.id).collect();
        assert!(held.is_empty(), "{id}: title {} holds {held:?}", node.id);
        let leaf = match id {
            "alloy" | "simple-x" | "xglow" => Some("UserNameText"),
            "xslimfire" => Some("ChatUserNameText"),
            _ => None,
        };
        if let Some(leaf) = leaf {
            assert_eq!(node.id, leaf, "{id}: the title is the pack's text");
        }
        let area = one(t, w, h, NodeRole::Slot(SlotKey::ChatArea), id);
        let [left, top, wide, tall] = title;
        assert!(
            wide > 0 && left >= 0 && left + wide <= w && top >= 0 && top + tall <= area[1],
            "{id}: title {title:?} outside the strip above {area:?}"
        );
        for logo in find(t, w, h, NodeRole::Logo) {
            assert!(
                !super::validate::overlap(title, logo),
                "{id}: title {title:?} over the logo {logo:?}"
            );
        }
    }
}

/// Each node depth first with its (z, sibling index) at every level.
fn stacks(nodes: &[SkinNode], at: &mut Vec<(i16, usize)>, out: &mut Vec<Vec<(i16, usize)>>) {
    for (i, n) in nodes.iter().enumerate() {
        at.push((n.z, i));
        out.push(at.clone());
        stacks(&n.children, at, out);
        at.pop();
    }
}

/// Plan SK17 (round 34: Alloy's blank pane, Simple X's missing presence
/// row): the message list and the composer are the engine's child
/// windows, drawn over the skin's painted tiles, so each paints above
/// every art node its box meets (Alloy's `MainTitleBar` and `Middle`,
/// Simple X's 70 px title bar art), at the served minimum and 1200x900.
/// Every node is its own stacking level, so the order is decided where
/// the two nodes' ancestries part.
#[test]
fn the_chat_area_and_composer_draw_above_the_art_they_meet() {
    for id in PACKS {
        let l = chat_layout(id);
        let t = l.tree.as_ref().unwrap();
        let mut order = Vec::new();
        stacks(&t.root, &mut Vec::new(), &mut order);
        for (w, h) in [min_window(&l), (1200, 900)] {
            let boxes = t.boxes(w, h);
            for (s, (slot, sb)) in boxes.iter().enumerate() {
                if !matches!(
                    slot.role,
                    NodeRole::Slot(SlotKey::ChatArea | SlotKey::ChatEdit)
                ) {
                    continue;
                }
                for (a, (art, ab)) in boxes.iter().enumerate() {
                    let (ps, pa) = (&order[s], &order[a]);
                    let Some(d) = ps.iter().zip(pa).position(|(x, y)| x != y) else {
                        continue;
                    };
                    if art.art.is_some() && super::validate::overlap(*sb, *ab) {
                        assert!(
                            ps[d] > pa[d],
                            "{id} at {w}x{h}: {} {sb:?} under {} {ab:?}",
                            slot.id,
                            art.id
                        );
                    }
                }
            }
        }
    }
}

/// Plan SK17 (round 34: Combat's frame clipped at the left): Combat's
/// `[Top]` lies between its corners (`X=0` then `X=TopLeft.RIGHT`,
/// `IndentRight=TopRight.width`, `CombatChatSkin.ini:24-31`), never over
/// `TopLeft`, and every frame tile lies within the window.
#[test]
fn combats_top_lies_between_its_corners() {
    let l = chat_layout("combat");
    let t = l.tree.as_ref().unwrap();
    for (w, h) in [min_window(&l), (1200, 900)] {
        let (left, _) = by_id(t, w, h, "TopLeft");
        let (top, _) = by_id(t, w, h, "Top");
        let (right, _) = by_id(t, w, h, "TopRight");
        assert!(
            top[0] >= left[0] + left[2] && top[0] + top[2] <= right[0],
            "combat at {w}x{h}: Top {top:?} not between {left:?} and {right:?}"
        );
        for (n, [x, _, bw, _]) in t.boxes(w, h) {
            if n.role == NodeRole::Decor && n.art.is_some() {
                assert!(
                    x >= 0 && x + bw <= w,
                    "combat at {w}x{h}: {} at x {x} w {bw}",
                    n.id
                );
            }
        }
    }
}

/// One line of composer text: the shell's 12 px base font
/// (`src/styles/global.css:289`).
const LINE: i32 = 12;

/// Plan SKW3 (lead ruling b): every built-in's chat and group-chat
/// composer edit is at least one text line tall at the served minimum.
/// xfire-classic's `ChatEdit` (`Y=Divider.BOTTOM ResizeY=100
/// IndentBottom=19`, `xfirechatwindow.xml:211-238`) was served 2 px tall
/// from a package written before shares chained.
#[test]
fn every_composer_edit_holds_a_text_line() {
    let mut edits = 0;
    for b in &builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let served = manifest::served(&package).unwrap();
        for window in [SkinWindow::Chat, SkinWindow::Community] {
            let Some(layout) = served.windows.get(&window) else {
                continue;
            };
            let Some(tree) = layout.tree.as_ref() else {
                continue;
            };
            let (width, height) = min_window(layout);
            for [_, _, _, edit_h] in find(tree, width, height, NodeRole::Slot(SlotKey::ChatEdit)) {
                edits += 1;
                assert!(
                    edit_h >= LINE,
                    "{} {window:?} at {width}x{height}: the composer edit is {edit_h} px",
                    b.id
                );
            }
        }
    }
    assert!(edits > 0);
}
