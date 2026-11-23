//! Each built-in's main-window component tree against its golden (plan
//! SK2-S1f step 4): the nodes in document order, one per line, indented by
//! depth, with their roles. The goldens are reviewed against design
//! `sk2/design.md` §2 (the per-skin component tables).

use std::fmt::Write as _;

use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeRole, ShellNode, SkinNode, SkinTree};

use crate::{builtin, images, manifest, package};

fn lines(nodes: &[SkinNode], depth: usize, out: &mut String) {
    for n in nodes {
        let _ = write!(out, "{}{} {:?}", "  ".repeat(depth), n.id, n.role);
        if let Some(l) = &n.label {
            let _ = write!(out, " label {:?}", l.text);
            if let Some(c) = l.cover {
                let _ = write!(out, " cover {c:?}");
            }
            if let Some(g) = l.glyph {
                let _ = write!(out, " glyph {g:?}");
            }
        }
        if let Some(i) = n.inset {
            let _ = write!(out, " inset [{}, {}]", i.left, i.right);
        }
        if let Some(t) = n.target {
            let _ = write!(out, " target {:?} art {:?} {:?}", t.area, t.art, t.rule);
        }
        out.push('\n');
        lines(&n.children, depth + 1, out);
    }
}

/// The built-in's main window layout, its package validated.
fn main_layout(id: &str) -> WindowLayout {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let package = package::read(b.bytes).unwrap();
    let mut m = manifest::served(&package).unwrap();
    images::validate(&package, &m).unwrap();
    m.windows.remove(&SkinWindow::Main).unwrap()
}

/// The main window's tree and the trees of the popups it launches (plan
/// SK2 P1: StealthX's menu titles and `AddFriendButton` ride the
/// `MainMenuPopup` rail, `popups.xml:2-89`).
fn main_and_popup_trees(id: &str) -> Vec<SkinTree> {
    let main = main_layout(id);
    std::iter::once(main.tree.unwrap())
        .chain(main.popups.into_values().map(|p| p.tree))
        .collect()
}

/// The main tree, then each popup tree it launches under a `popup <name>`
/// header (plan SK2 P1: a node the pack moved onto a popup stays golden).
fn tree_of(id: &str) -> String {
    let main = main_layout(id);
    let mut out = String::new();
    lines(&main.tree.as_ref().unwrap().root, 0, &mut out);
    for (name, popup) in &main.popups {
        let _ = writeln!(out, "popup {name}");
        lines(&popup.tree.root, 1, &mut out);
    }
    out
}

#[test]
fn builtin_trees_match_their_goldens() {
    let want = [
        ("symbiosis", include_str!("goldens/symbiosis.txt")),
        ("xfire-classic", include_str!("goldens/xfire-classic.txt")),
        ("combat", include_str!("goldens/combat.txt")),
        ("alloy", include_str!("goldens/alloy.txt")),
        ("onyx", include_str!("goldens/onyx.txt")),
        ("simple-x", include_str!("goldens/simple-x.txt")),
        ("spacecowboy", include_str!("goldens/spacecowboy.txt")),
        ("stealthx", include_str!("goldens/stealthx.txt")),
        ("total-chaos", include_str!("goldens/total-chaos.txt")),
        ("xglow", include_str!("goldens/xglow.txt")),
        ("xslimfire", include_str!("goldens/xslimfire.txt")),
    ];
    let mut wrong = Vec::new();
    for (id, golden) in want {
        let got = tree_of(id);
        if got != golden {
            wrong.push(format!("=== {id}\n{got}=== end {id}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

fn find_all<'a>(nodes: &'a [SkinNode], id: &str, out: &mut Vec<&'a SkinNode>) {
    for n in nodes {
        if n.id == id {
            out.push(n);
        }
        find_all(&n.children, id, out);
    }
}

/// Plan SK2-A3 (design §2.4): the behaviour roles collapsed into
/// `Button`/`MenuTitle`/`Part` + triggers, and no control lost its
/// behaviour. `goldens/former_behaviour.txt` lists every node of the 11
/// built-ins' main trees that had a behaviour role before the cut
/// (`Minimize`, `Close`, `Menu(_)`, `Resize(_)`, `Button(_)`, …, taken from
/// the goldens at `aa6d776cd`); each still carries a pressing trigger (its
/// own or its part's, the pack's wiring), and a former resize grip still
/// resizes. A node the pack moved into a popup the main window launches is
/// searched for there.
#[test]
fn every_former_behaviour_node_keeps_a_trigger() {
    let mut lost = Vec::new();
    let mut trees: std::collections::BTreeMap<&str, Vec<SkinTree>> =
        std::collections::BTreeMap::new();
    for line in include_str!("goldens/former_behaviour.txt").lines() {
        let mut f = line.split(' ');
        let (skin, id, old) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let skin_trees = trees
            .entry(skin)
            .or_insert_with(|| main_and_popup_trees(skin));
        let mut found = Vec::new();
        for tree in skin_trees.iter() {
            find_all(&tree.root, id, &mut found);
        }
        // Alloy's `IVButtonHolder` was misread as the InfoView toggle: it is
        // `Class=Tile` art with no trigger (`Chatskin.ini:523-527`).
        if (skin, id) == ("alloy", "IVButtonHolder") {
            assert!(found
                .iter()
                .all(|n| n.role == NodeRole::Decor && n.triggers.is_empty()));
            continue;
        }
        if found.is_empty() {
            lost.push(format!("{skin} {id} ({old}): gone"));
        }
        for n in found {
            // A former grip still resizes: from the pack's own edge where
            // it wires one (Symbiosis's `ResizeGripperTL` runs `ResizeUL`,
            // which the name table had read as bottom-right).
            let grip = !old.starts_with("Resize(") || n.resize_edge().is_some();
            if n.presses().next().is_none() || !grip {
                lost.push(format!(
                    "{skin} {id} ({old}): {:?} {:?}",
                    n.role, n.triggers
                ));
            }
        }
    }
    assert!(lost.is_empty(), "{}", lost.join("\n"));
}

/// The pack's `ControlGroupId` rides the node (design §2.6): Classic's
/// game-button container `ButtonArea` and its tab control `TabbedControl`
/// both carry group 1 (`xfirecomponents.xml:1172`), so its rows switch
/// with the tabs, and the served shell carries the same group.
#[test]
fn classics_button_container_carries_its_control_group() {
    let tree = main_layout("xfire-classic").tree.unwrap();
    for id in ["ButtonArea", "TabbedControl"] {
        let mut found = Vec::new();
        find_all(&tree.root, id, &mut found);
        assert_eq!(found.len(), 1, "{id}");
        assert_eq!(found[0].control_group, Some(1), "{id}");
    }
    fn shell_node<'a>(nodes: &'a [ShellNode], id: &str) -> Option<&'a ShellNode> {
        nodes.iter().find_map(|n| {
            (n.id == id)
                .then_some(n)
                .or_else(|| shell_node(&n.children, id))
        })
    }
    let shell = tree.shell(SkinWindow::Main, (0, 0), Vec::new());
    let area = shell_node(&shell.root, "ButtonArea").unwrap();
    assert_eq!(area.control_group, Some(1));
}

fn menu_label(id: &str, node: &str) -> Option<rekindle_types::dto::skin_tree::NodeLabel> {
    let trees = main_and_popup_trees(id);
    let mut found = Vec::new();
    for t in &trees {
        find_all(&t.root, node, &mut found);
    }
    assert_eq!(found.len(), 1, "{id} {node}");
    found[0].label.clone()
}

/// Plan SK4b, lead decision 7: Symbiosis's file title art bakes "Xfire"
/// (47 x 20 `node-xfiremenu-topleft.png`), so its label is "Rekindle"; its
/// Tools and Help art reads right and carries no label; Xfire Classic
/// draws its titles as text, so its file title is "Rekindle" with no
/// cover. Plan SK5: the cover is the title's usable box, not the word's
/// 22 px ("Re..."): the art draws no frame around "Xfire" (x 12-33,
/// y 4-13), so the cover is the whole 45 x 20 node, the word inside it.
#[test]
fn menu_titles_carry_the_hosts_words() {
    let file = menu_label("symbiosis", "XfireMenu").expect("file label");
    assert_eq!(file.text, "Rekindle");
    let [x, y, w, h] = file.cover.expect("cover");
    assert!(
        w > 0 && h > 0 && x + w <= 45 && y + h <= 20,
        "{:?}",
        file.cover
    );
    let word = [12, 4, 22, 10];
    assert!(
        x <= word[0] && y <= word[1] && x + w >= word[0] + word[2] && y + h >= word[1] + word[3],
        "cover {:?} misses the word {word:?}",
        file.cover
    );
    assert!(
        w >= 45,
        "cover {:?} narrower than the 45 px frameless node",
        file.cover
    );
    assert_eq!(menu_label("symbiosis", "ToolsMenu"), None);
    assert_eq!(menu_label("symbiosis", "HelpMenu"), None);
    assert_eq!(file.glyph, None, "symbiosis's 45 x 20 title is worded");
    let classic = menu_label("xfire-classic", "XfireMenuText").expect("text label");
    assert_eq!((classic.text.as_str(), classic.cover), ("Rekindle", None));
}

/// Plan SK4b: on each art-titled pack the file cover hides the whole baked
/// word. The word boxes are read by eye off each pack's art, in the title's
/// box: Symbiosis "Xfire" x 12-33, y 4-13; Alloy "XFIRE" x 2-45, y 2-12;
/// Total Chaos "XFIRE" x 15-49, y 3-9 (its `MenuBg` from x 11, y 12);
/// XGlow "File" x 2-19, y 2-9 and its mirrored reflection to x 21, y 17.
/// Their Tools and Help art reads right and carries no label; the
/// text-drawn packs carry `{text}` on all three titles.
#[test]
fn file_covers_hide_the_whole_baked_word() {
    let holds = |c: [u16; 4], w: [u16; 4]| {
        c[0] <= w[0] && c[1] <= w[1] && c[0] + c[2] >= w[0] + w[2] && c[1] + c[3] >= w[1] + w[3]
    };
    for (pack, file, word, tools) in [
        ("symbiosis", "XfireMenu", [12, 4, 22, 10], "ToolsMenu"),
        ("alloy", "FileMenu", [2, 2, 44, 11], "ToolsMenu"),
        ("total-chaos", "FileMenu", [15, 3, 35, 7], "ToolsMenu"),
        ("xglow", "FileMenu", [2, 2, 20, 16], "ToolMenu"),
    ] {
        let label = menu_label(pack, file).expect(pack);
        assert_eq!(label.text, "Rekindle", "{pack}");
        let cover = label.cover.expect(pack);
        assert!(
            holds(cover, word),
            "{pack}: cover {cover:?} misses the word {word:?}"
        );
        assert!(
            cover[2] >= word[2],
            "{pack}: cover {cover:?} narrower than its word"
        );
        assert_eq!(menu_label(pack, tools), None, "{pack} tools");
        assert_eq!(menu_label(pack, "HelpMenu"), None, "{pack} help");
    }
    for (pack, file) in [
        ("xfire-classic", "XfireMenuText"),
        ("combat", "FileMenuText"),
        ("stealthx", "XfireMenuText"),
        ("onyx", "XfireMenuText"),
        ("simple-x", "XfireMenuText"),
        ("spacecowboy", "XfireMenuText"),
        ("xslimfire", "XfireMenuText"),
    ] {
        for (node, text) in [
            (file, "Rekindle"),
            ("ToolsMenuText", "Tools"),
            ("HelpMenuText", "Help"),
        ] {
            let label = menu_label(pack, node).expect(pack);
            assert_eq!(
                (label.text.as_str(), label.cover, label.glyph),
                (text, None, None),
                "{pack} {node}"
            );
        }
    }
}

fn name_field(id: &str) -> SkinNode {
    let tree = main_layout(id).tree.unwrap();
    let mut stack: Vec<&SkinNode> = tree.root.iter().collect();
    while let Some(n) = stack.pop() {
        if n.role == NodeRole::NameField {
            return n.clone();
        }
        stack.extend(&n.children);
    }
    panic!("{id}: no name field");
}

/// Plan SK5: Symbiosis lays its notifications gear `ButtonNotificationsTab`
/// (x 309-334 at the 355 px minimum) over the end of its name field
/// `UsernameContainer` (x 7-336), so the field serves a right inset of at
/// least the gear's reach, 337 - 309 = 28 px, and the shell carries it.
#[test]
fn a_name_field_under_a_button_serves_its_inset() {
    let field = name_field("symbiosis");
    let inset = field.inset.expect("symbiosis name field inset");
    assert!(inset.right >= 28, "{inset:?}");
    assert_eq!(inset.left, 0, "{inset:?}");
    let tree = main_layout("symbiosis").tree.unwrap();
    let shell = tree.shell(SkinWindow::Main, (0, 0), Vec::new());
    let mut stack: Vec<&ShellNode> = shell.root.iter().collect();
    let mut served = None;
    while let Some(n) = stack.pop() {
        if n.id == field.id {
            served = n.inset;
        }
        stack.extend(&n.children);
    }
    assert_eq!(served, Some(inset));
}

/// Plan SK5: a name field no button, tab or menu meets serves no inset
/// (Alloy's `UserNameField`; its status field is a text field, never a
/// covering sibling).
#[test]
fn a_clear_name_field_serves_no_inset() {
    assert_eq!(name_field("alloy").inset, None);
    assert_eq!(name_field("xfire-classic").inset, None);
}

/// Plan SK5 (lead decision): XGlow's file title is icon-shaped (one whole
/// 24 x 22 picture, at most 2:1), and its usable box (21 px) holds no
/// legible "Rekindle", so it carries the app mark glyph over that box, the
/// words its accessible name.
#[test]
fn an_icon_shaped_renamed_title_carries_the_app_mark() {
    let label = menu_label("xglow", "FileMenu").expect("xglow file label");
    assert_eq!(label.text, "Rekindle");
    assert_eq!(
        label.glyph,
        Some(rekindle_types::dto::skin_tree::NodeGlyph::Rekindle)
    );
    assert_eq!(label.cover, Some([1, 0, 21, 22]));
}

/// Plan SK6: a tab the pack draws as text (Classic's `TabNormalText`,
/// `Class="Text"`, its word a string of the Xfire binary) carries the
/// Host's name for its view, no cover and no glyph; a tab whose art draws
/// its word carries none.
#[test]
fn text_tabs_carry_the_hosts_words() {
    let words = |id: &str, node: &str| {
        menu_label(id, node).map(|l| {
            assert_eq!((l.cover, l.glyph), (None, None), "{id} {node}");
            l.text
        })
    };
    for id in ["xfire-classic", "onyx", "simple-x"] {
        assert_eq!(
            words(id, "TabNormalText").as_deref(),
            Some("Friends"),
            "{id}"
        );
        assert_eq!(
            words(id, "TabServerText").as_deref(),
            Some("Servers"),
            "{id}"
        );
        assert_eq!(
            words(id, "TabDownloadText").as_deref(),
            Some("Downloads"),
            "{id}"
        );
    }
    for (id, node) in [
        ("alloy", "TabNormalButton"),
        ("total-chaos", "TabServerButton"),
        ("symbiosis", "ButtonFriendsTab"),
        ("xglow", "Downloads"),
    ] {
        assert_eq!(menu_label(id, node), None, "{id} {node}");
    }
}

/// Plan SK6b: a button's text component (StealthX's `ChatButtonText`,
/// Classic's `BigBrowseFileText`: `Class="Text"`, its word a string of the
/// Xfire binary) carries the Host's words for the action the button runs,
/// no cover and no glyph.
#[test]
fn button_text_parts_carry_the_hosts_words() {
    let words = |id: &str, node: &str| {
        menu_label(id, node).map(|l| {
            assert_eq!((l.cover, l.glyph), (None, None), "{id} {node}");
            l.text
        })
    };
    let want = [
        ("xfire-classic", "BigBrowseFileText", "Find"),
        ("simple-x", "BigBrowseFileText", "Find"),
        ("stealthx", "AddFriendButtonText", "Add Friend"),
        ("stealthx", "ChatButtonText", "Chat"),
        ("stealthx", "GameButton1.GameButtonText", "Join"),
        ("stealthx", "InfoButton1.InfoButtonText", "Profile"),
        ("stealthx", "AddServerButtonText", "Add Server"),
        ("stealthx", "RefreshButtonText", "Refresh"),
        ("stealthx", "GameButton2.GameButtonText", "Join"),
        ("stealthx", "InfoButton2.InfoButtonText", "Profile"),
        ("stealthx", "FindButtonText", "Find"),
        ("stealthx", "DownloadButtonText", "Start"),
        ("stealthx", "StopButtonText", "Stop"),
        ("stealthx", "ExecuteButtonText", "Execute"),
        ("stealthx", "HostChatButtonText", "Host Voice"),
        ("stealthx", "JoinChatButtonText", "Join Voice"),
        ("stealthx", "LeaveChatButtonText", "Leave Voice"),
    ];
    for (id, node, text) in want {
        assert_eq!(words(id, node).as_deref(), Some(text), "{id} {node}");
    }
}

/// Plan SK6b: a button whose art draws its word (Classic's `AddButton`,
/// `ChatButton`) serves no words, nor does a button's art-less state tile
/// (the info popup's `TalkingStateButton`).
#[test]
fn art_worded_buttons_serve_no_words() {
    for (id, node) in [
        ("xfire-classic", "AddButton"),
        ("xfire-classic", "ChatButton"),
        ("xfire-classic", "InfoTalkingStateButton"),
        ("alloy", "TalkingStateButton"),
    ] {
        assert_eq!(menu_label(id, node), None, "{id} {node}");
    }
}

/// Plan SK6b: across the 11 built-ins, no text component of a button is
/// left blank: every art-less `…Text` part under a button carries words,
/// so every pressed action of such a button has a name.
#[test]
fn no_button_text_part_is_left_blank() {
    fn blank(nodes: &[SkinNode], under_button: bool, id: &str, out: &mut Vec<String>) {
        for n in nodes {
            let text = n.id.to_ascii_lowercase().ends_with("text");
            let part = under_button && n.role == NodeRole::Part && n.art.is_none();
            if part && text && n.label.as_ref().is_none_or(|l| l.text.is_empty()) {
                out.push(format!("{id} {}", n.id));
            }
            blank(&n.children, n.role == NodeRole::Button, id, out);
        }
    }
    let mut left = Vec::new();
    for id in builtin::PACKS.iter().map(|b| b.id) {
        for t in main_and_popup_trees(id) {
            blank(&t.root, false, id, &mut left);
        }
    }
    assert!(left.is_empty(), "{left:?}");
}
