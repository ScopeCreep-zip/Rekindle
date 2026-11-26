//! Plan E7.7, RV1: the text-over-art rule ([`super::text_top`]). Alloy's
//! and Simple X's `StatusWrapper` plate (`Z=4`) lies over their
//! `StatusEdit` field (`Z=3`, `Alloy.ini:45–64`, `SimpleX.ini:47–70`); the
//! field is served above the plate and below the `DownArrow` button
//! (`Z=5`). Onyx's field (`Z=22`) already lies above its plate (18).

use rekindle_types::dto::skin_tree::{SkinNode, SkinTree};

use super::golden_tests::main_layout;
use super::test_support::find;
use super::text_top::{boxes, meets, shows_text, subtree, takes_input};
use super::validate::{Apart, Stack};
use super::{popup, target};
use crate::{builtin, manifest, package};

fn status_stack(id: &str) -> (i16, bool, bool) {
    let tree = main_layout(id).tree.unwrap();
    let stack = Stack::new(&tree.root);
    let field = find(&tree.root, "StatusEdit").unwrap();
    (
        field.z,
        stack.above("StatusEdit", "StatusWrapper"),
        stack.above("DownArrow", "StatusEdit"),
    )
}

#[test]
fn alloy_and_simple_x_status_field_above_its_plate_below_its_arrow() {
    for id in ["alloy", "simple-x"] {
        let (z, over_plate, under_arrow) = status_stack(id);
        assert!(over_plate, "{id}: StatusEdit (Z {z}) under StatusWrapper");
        assert!(under_arrow, "{id}: StatusEdit (Z {z}) over DownArrow");
    }
}

#[test]
fn onyx_status_field_keeps_its_pack_order() {
    let tree = main_layout("onyx").tree.unwrap();
    let z = |id: &str| find(&tree.root, id).unwrap().z;
    assert_eq!((z("StatusEdit"), z("StatusWrapper")), (22, 18));
}

/// Each node with its ancestors, root first.
fn chains<'a>(nodes: &'a [SkinNode], up: &[&'a SkinNode], out: &mut Vec<Vec<&'a SkinNode>>) {
    for n in nodes {
        let mut own = up.to_vec();
        own.push(n);
        chains(&n.children, &own, out);
        out.push(own);
    }
}

/// Every text node of `tree` laid in `at` that an art-only node stacked
/// above it covers: the art's branch below their common ancestor takes
/// no input (an interactive branch keeps its pack order).
fn covered(name: &str, tree: &SkinTree, at: (i32, i32), out: &mut Vec<String>) {
    let laid = boxes(tree, at);
    let stack = Stack::new(&tree.root);
    let apart = Apart::new(&tree.root);
    let mut all = Vec::new();
    chains(&tree.root, &[], &mut all);
    for t in all.iter().filter(|c| shows_text(c[c.len() - 1])) {
        let text = t[t.len() - 1];
        for a in &all {
            let art = a[a.len() - 1];
            let common = t
                .iter()
                .zip(a.iter())
                .take_while(|(x, y)| x.id == y.id)
                .count();
            let kin = common == t.len() || common == a.len();
            if kin || art.art.is_none() || takes_input(art) {
                continue;
            }
            let branch = a[common];
            if subtree(branch).iter().any(|n| takes_input(n))
                || apart.apart(&text.id, &art.id)
                || !meets(&laid, &text.id, &art.id)
                || !stack.above(&art.id, &text.id)
            {
                continue;
            }
            out.push(format!("{name}: {} covered by {}", text.id, art.id));
        }
    }
}

#[test]
fn no_text_control_lies_under_art_only_nodes_in_any_builtin() {
    let mut out = Vec::new();
    for b in builtin::PACKS {
        let m = manifest::served(&package::read(b.bytes).unwrap()).unwrap();
        for (window, l) in &m.windows {
            if let Some(t) = &l.tree {
                covered(
                    &format!("{} {window:?}", b.id),
                    t,
                    target::window(l),
                    &mut out,
                );
            }
            for (p, pop) in &l.popups {
                let at = target::window(&popup::frame(pop, l.min_height));
                covered(
                    &format!("{} {window:?} popup {p}", b.id),
                    &pop.tree,
                    at,
                    &mut out,
                );
            }
        }
    }
    assert!(out.is_empty(), "{} covered:\n{}", out.len(), out.join("\n"));
}
