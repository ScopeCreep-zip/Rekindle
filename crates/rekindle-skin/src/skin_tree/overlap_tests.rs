//! Plan SK12: the served boxes of the eleven built-ins never overlap, laid
//! at each window's served minimum: no two press areas, and no text box over
//! another node's press area or text box, except a node's own descendants;
//! none of them over the views (the app's content: the buddy list, its
//! search field). Two press areas never overlap, even where the pack's arts
//! do (lead decision: the node drawn above keeps its press area and the one
//! below loses the part it covers, `target::under`).
//! A text box never overlaps another node's press area or text box either,
//! even where the pack's arts overlap (plan SK13: the text box stops at it,
//! `text_box::usable`).

use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree};

use super::inset::is_text;
use super::target::{listens, meet, outer};
use super::validate::{overlap, Apart, HIT};
use super::{popup, target, text_box};
use crate::{builtin, manifest, package};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Press,
    Text,
    Views,
}

/// One served box: the node, its ancestors' ids, its art box, the box.
struct Served<'a> {
    node: &'a SkinNode,
    up: Vec<&'a str>,
    art: [i32; 4],
    area: [i32; 4],
    kind: Kind,
}

fn walk<'a>(nodes: &'a [SkinNode], parent: [i32; 4], up: &[&'a str], out: &mut Vec<Served<'a>>) {
    for n in nodes {
        let b = super::titles::place(n, parent);
        let seen = b[2] > 0 && b[3] > 0;
        let mut push = |area: [i32; 4], kind| {
            out.push(Served {
                node: n,
                up: up.to_vec(),
                art: b,
                area,
                kind,
            });
        };
        if seen && listens(n) && !n.frame_node() {
            push(n.target.map_or(b, |t| outer(b, t)), Kind::Press);
        }
        // A text the pack lays 0 px tall draws a line from its top (Combat's
        // name `Title`): its rows, at least one, as the text pass lays it.
        // A text field with art types its text over the art less its inset
        // (Alloy's 205 px name field).
        if b[2] > 0 && (text_box::draws_text(n) || is_text(n.role)) {
            let i = n
                .inset
                .map_or((0, 0), |i| (i32::from(i.left), i32::from(i.right)));
            let t = [b[0] + i.0, b[1], b[2] - i.0 - i.1, b[3].max(1)];
            if t[2] > 0 {
                push(t, Kind::Text);
            }
        }
        if seen && n.role == NodeRole::Views {
            push(b, Kind::Views);
        }
        let mut own = up.to_vec();
        own.push(n.id.as_str());
        walk(&n.children, b, &own, out);
    }
}

/// The overlaps of one served tree laid in `at`, one line each.
fn overlaps(name: &str, tree: &SkinTree, at: (i32, i32), out: &mut Vec<String>) {
    let mut served = Vec::new();
    walk(
        &tree.root,
        super::titles::laid_window(tree.outsets, at),
        &[],
        &mut served,
    );
    let apart = Apart::new(&tree.root);
    for (i, a) in served.iter().enumerate() {
        // An undersized press area the Host serves no rule for.
        let small = a.area[2] < HIT || a.area[3] < HIT;
        if a.kind == Kind::Press && small && a.node.target.is_none() {
            out.push(format!(
                "{name}: {} {:?} undersized, no rule",
                a.node.id, a.area
            ));
        }
        for b in &served[i + 1..] {
            let (x, y) = (a.node.id.as_str(), b.node.id.as_str());
            let kin = x == y || a.up.contains(&y) || b.up.contains(&x);
            let pair = (a.kind, b.kind);
            let compared = !matches!(pair, (Kind::Views, Kind::Views));
            if kin || !compared || apart.apart(x, y) {
                continue;
            }
            // Over the views, a box reaches no further than the art the
            // pack lays there (Simple-X's menu buttons end 5 px into it).
            let views = matches!(pair, (Kind::Views, _) | (_, Kind::Views));
            let over = if views {
                let (o, v) = if a.kind == Kind::Views {
                    (b, a)
                } else {
                    (a, b)
                };
                meet(o.area, v.area).is_some_and(|m| meet(m, o.art) != Some(m))
            } else {
                overlap(a.area, b.area)
            };
            if over {
                out.push(format!(
                    "{name}: {x} {:?} {:?} over {y} {:?} {:?}",
                    a.kind, a.area, b.kind, b.area
                ));
            }
        }
    }
}

/// Every built-in window and popup, at its served minimum.
fn all_overlaps() -> Vec<String> {
    let mut out = Vec::new();
    // Plan SK14: the accessible alternate, laid from Symbiosis's bytes at
    // its own address, keeps the invariant too.
    let symbiosis = builtin::find(crate::conform::SOURCE).unwrap().1.bytes;
    let alternate = crate::conform::served(
        symbiosis,
        &package::read(symbiosis).unwrap(),
        crate::conform::address(),
    )
    .unwrap();
    let served = builtin::PACKS
        .iter()
        .map(|b| {
            (
                b.id,
                manifest::served(&package::read(b.bytes).unwrap()).unwrap(),
            )
        })
        .chain(std::iter::once(("symbiosis-access", alternate)));
    for (id, m) in served {
        for (window, l) in &m.windows {
            if let Some(t) = &l.tree {
                overlaps(&format!("{id} {window:?}"), t, target::window(l), &mut out);
            }
            for (p, pop) in &l.popups {
                let at = target::window(&popup::frame(pop, l.min_height));
                overlaps(
                    &format!("{id} {window:?} popup {p}"),
                    &pop.tree,
                    at,
                    &mut out,
                );
            }
        }
    }
    out
}

#[test]
fn no_served_press_or_text_box_overlaps_another() {
    let got = all_overlaps();
    assert!(
        got.is_empty(),
        "{} overlaps:\n{}",
        got.len(),
        got.join("\n")
    );
}
