//! Each built-in's main-window component tree against its golden (plan
//! SK2-S1f step 4): the nodes in document order, one per line, indented by
//! depth, with their roles. The goldens are reviewed against design
//! `sk2/design.md` §2 (the per-skin component tables).

use std::fmt::Write as _;

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_tree::SkinNode;

use crate::{builtin, images, manifest, package};

fn lines(nodes: &[SkinNode], depth: usize, out: &mut String) {
    for n in nodes {
        let _ = writeln!(out, "{}{} {:?}", "  ".repeat(depth), n.id, n.role);
        lines(&n.children, depth + 1, out);
    }
}

fn tree_of(id: &str) -> String {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let package = package::read(b.bytes).unwrap();
    let m = manifest::parse(&package).unwrap();
    images::validate(&package, &m).unwrap();
    let tree = m.windows[&SkinWindow::Main].tree.as_ref().unwrap();
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
