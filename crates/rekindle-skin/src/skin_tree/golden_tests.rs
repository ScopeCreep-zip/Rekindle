//! Each built-in's main-window component tree against its golden (plan
//! SK2-S1f step 4): the nodes in document order, one per line, indented by
//! depth, with their roles. The goldens are reviewed against design
//! `sk2/design.md` §2 (the per-skin component tables).

use std::fmt::Write as _;

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree};

use crate::{builtin, images, manifest, package};

fn lines(nodes: &[SkinNode], depth: usize, out: &mut String) {
    for n in nodes {
        let _ = writeln!(out, "{}{} {:?}", "  ".repeat(depth), n.id, n.role);
        lines(&n.children, depth + 1, out);
    }
}

fn main_tree(id: &str) -> SkinTree {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let package = package::read(b.bytes).unwrap();
    let m = manifest::parse(&package).unwrap();
    images::validate(&package, &m).unwrap();
    m.windows[&SkinWindow::Main].tree.clone().unwrap()
}

fn tree_of(id: &str) -> String {
    let tree = main_tree(id);
    let mut out = String::new();
    lines(&tree.root, 0, &mut out);
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
/// resizes.
#[test]
fn every_former_behaviour_node_keeps_a_trigger() {
    let mut lost = Vec::new();
    let mut trees: std::collections::BTreeMap<&str, SkinTree> = std::collections::BTreeMap::new();
    for line in include_str!("goldens/former_behaviour.txt").lines() {
        let mut f = line.split(' ');
        let (skin, id, old) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let tree = trees.entry(skin).or_insert_with(|| main_tree(skin));
        let mut found = Vec::new();
        find_all(&tree.root, id, &mut found);
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
