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
        let _ = writeln!(out, "{}{} {:?}", "  ".repeat(depth), n.id, n.role);
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
