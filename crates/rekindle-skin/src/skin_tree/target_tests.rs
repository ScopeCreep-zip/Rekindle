//! The served target box of a small interactive node (plan SK7, WCAG 2.2
//! SC 2.5.8): the node's own box is at least 24 x 24 around its art, or
//! the spacing exception holds; the `::before` hit area's promise (a 24 px
//! target) moved onto the node's box, where axe measures it.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{NodeRole, NodeTarget, SkinNode, SkinTree, TargetRule};

use super::test_support::Z;
use super::tests::{acts, fixed, layout, n};
use super::{css, target, validate};
use crate::{builtin, images, manifest, package};

fn button(id: &str, (x, y): (i16, i16), (w, h): (u16, u16)) -> SkinNode {
    acts(
        n(id, NodeRole::Button, fixed(x, w), fixed(y, h)),
        SkinEvent::Click,
        SkinAction::Minimize,
    )
}

/// A 200 x 300 window holding `root`, its targets set as the Host serves.
fn laid(root: Vec<SkinNode>) -> WindowLayout {
    let mut l = layout(Some(SkinTree { outsets: Z, root }));
    target::nodes(&mut l);
    l
}

fn target_of(l: &WindowLayout, id: &str) -> Option<NodeTarget> {
    let tree = l.tree.as_ref().unwrap();
    tree.root.iter().find(|n| n.id == id).unwrap().target
}

fn sheet(l: &WindowLayout) -> String {
    css::tree_css(".f", l.tree.as_ref().unwrap(), &BTreeMap::new(), "x").unwrap()
}

#[test]
fn an_18_px_titlebar_button_serves_a_24_px_target_around_its_art() {
    let l = laid(vec![button("Close", (50, 10), (18, 18))]);
    assert_eq!(
        target_of(&l, "Close"),
        Some(NodeTarget {
            area: [-3, -3, 24, 24],
            art: [3, 3, 18, 18],
            rule: TargetRule::Size,
        })
    );
    let out = sheet(&l);
    // The node's own box is the 24 px target; the art stays 18 x 18 inside
    // it (a transparent border of the inset), and no pseudo-element.
    assert!(out.contains("[data-skin-node=\"Close\"] {\n    position: absolute; left: 50px; width: 18px; top: 10px; height: 18px; z-index: 0; box-sizing: border-box; margin: -3px -3px -3px -3px; border: solid transparent; border-width: 3px 3px 3px 3px; width: 24px; height: 24px;"), "{out}");
    assert!(!out.contains("\"Close\"]::before"), "{out}");
    // At the window's top-left corner it grows inward, the art at 0, 0.
    let l = laid(vec![button("Close", (0, 0), (18, 18))]);
    assert_eq!(
        target_of(&l, "Close").map(|t| (t.area, t.art)),
        Some(([0, 0, 24, 24], [0, 0, 18, 18]))
    );
    assert_eq!(
        validate::check("main", l.tree.as_ref().unwrap(), &l, &BTreeMap::new()),
        Ok(())
    );
}

#[test]
fn adjacent_small_buttons_never_overlap_and_each_meets_size_or_spacing() {
    // 6 px apart: each grows 3 px toward the other and they touch.
    let l = laid(vec![
        button("Min", (50, 10), (18, 18)),
        button("Close", (74, 10), (18, 18)),
    ]);
    let laid = target::targets(l.tree.as_ref().unwrap(), (200, 300));
    assert_eq!(laid[0].area, [47, 7, 24, 24]);
    assert_eq!(laid[1].area, [71, 7, 24, 24]);
    // 3 px from a 30 px button that never grows: the whole gap is its.
    let l = laid_pair((53, 10), (18, 18));
    assert_eq!(l[0].area, [30, 7, 24, 24]);
    assert_eq!(l[0].rule, TargetRule::Size);
    // 2 px from a 30 x 30 button's corner (2 px apart on both axes, cut on
    // x, the tie): 23 x 24, its 24 px circle clears the corner, so the
    // spacing exception holds.
    let corner = target::targets(
        &SkinTree {
            outsets: Z,
            root: vec![
                button("A", (30, 30), (18, 18)),
                button("B", (50, 50), (30, 30)),
            ],
        },
        (200, 300),
    );
    assert_eq!(corner[0].area, [27, 27, 23, 24]);
    assert_eq!(corner[0].rule, TargetRule::Spacing);
    assert!(!validate::overlap(corner[0].area, corner[1].area));
    // 2 px apart: each takes 1 px of the gap; the boxes touch, never
    // overlap, and neither size nor spacing holds (a finding, served).
    let tight = target::targets(
        &SkinTree {
            outsets: Z,
            root: vec![
                button("Min", (50, 10), (18, 18)),
                button("Close", (70, 10), (18, 18)),
            ],
        },
        (200, 300),
    );
    assert_eq!(tight[0].area, [47, 7, 22, 24]);
    assert_eq!(tight[1].area, [69, 7, 22, 24]);
    assert!(tight.iter().all(|t| t.rule == TargetRule::Short));
}

/// A small button at `at` beside a 30 x 40 button at x 0.
fn laid_pair(at: (i16, i16), size: (u16, u16)) -> Vec<target::Laid> {
    let tree = SkinTree {
        outsets: Z,
        root: vec![
            button("Big", (0, 0), (30, 40)),
            button("A", (at.0 - 20, at.1), size),
        ],
    };
    let mut l = target::targets(&tree, (200, 300));
    l.reverse();
    l
}

#[test]
fn a_node_already_24_px_is_unchanged() {
    let l = laid(vec![button("Big", (50, 10), (24, 30))]);
    assert_eq!(target_of(&l, "Big"), None);
    let out = sheet(&l);
    assert!(out.contains("[data-skin-node=\"Big\"] {\n    position: absolute; left: 50px; width: 24px; top: 10px; height: 30px; z-index: 0;\n  }"), "{out}");
}

/// Every built-in's interactive nodes, per window and popup: no two target
/// boxes overlap, and every node meets size or spacing except the listed
/// `Short` ones (plan SK7: a pack-layout finding, pinned here with numbers
/// and reported to the lead, never hidden; a new or a fixed one fails).
#[test]
fn every_builtin_interactive_node_meets_size_or_spacing() {
    let mut short = Vec::new();
    let mut seen = 0;
    for b in builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let m = manifest::served(&package).unwrap();
        images::validate(&package, &m).unwrap();
        for (window, l) in &m.windows {
            let mut trees = Vec::new();
            if let Some(t) = &l.tree {
                trees.push((String::new(), t, target::window(l)));
            }
            for (name, p) in &l.popups {
                let at = target::window(&super::popup::frame(p, l.min_height));
                trees.push((format!(" popup {name}"), &p.tree, at));
            }
            for (popup, tree, at) in trees {
                let laid = target::targets(tree, at);
                for g in target::grip_targets(tree, at, &laid) {
                    seen += 1;
                    if g.rule == TargetRule::Short {
                        short.push(format!(
                            "{} {window:?}{popup}: {} art {:?} target {:?} (grip)",
                            b.id, g.id, g.art, g.area
                        ));
                    }
                }
                for (i, t) in laid.iter().enumerate() {
                    seen += 1;
                    for u in &laid[i + 1..] {
                        let o =
                            validate::overlap(t.area, u.area) && !validate::overlap(t.art, u.art);
                        assert!(
                            !o,
                            "{} {window:?}{popup}: {} {:?} overlaps {} {:?}",
                            b.id, t.id, t.area, u.id, u.area
                        );
                    }
                    if t.rule == TargetRule::Short {
                        short.push(format!(
                            "{} {window:?}{popup}: {} art {:?} target {:?}",
                            b.id, t.id, t.art, t.area
                        ));
                    }
                }
            }
        }
    }
    assert!(seen > 0, "no interactive node to check");
    // The reviewed findings (plan SK7): each line a node whose pack lays a
    // neighbour too close for 24 px and for the spacing exception.
    let got = short.join("\n") + "\n";
    assert_eq!(got, include_str!("goldens/targets-short.txt"), "{got}");
}

/// Whether the shell marks `n` `data-skin-on` (`SkinTreeWindow.tsx`): a
/// pointer trigger of its own, or a role taking input.
fn shell_listens(n: &SkinNode) -> bool {
    n.triggers.iter().any(|t| t.on.pointer())
        || matches!(
            n.role,
            NodeRole::NameField
                | NodeRole::StatusText
                | NodeRole::StatusList
                | NodeRole::MenuBar
                | NodeRole::Views
                | NodeRole::View(_)
                | NodeRole::AdArea
                | NodeRole::InfoView
        )
}

/// Every node of `nodes` laid in `parent`: it, its box, its ancestors' ids.
fn laid_all<'a>(
    nodes: &'a [SkinNode],
    parent: [i32; 4],
    up: &[&'a str],
    out: &mut Vec<(&'a SkinNode, [i32; 4], Vec<&'a str>)>,
) {
    for n in nodes {
        let b = super::titles::place(n, parent);
        out.push((n, b, up.to_vec()));
        let mut path = up.to_vec();
        path.push(&n.id);
        laid_all(&n.children, b, &path, out);
    }
}

/// The box `tree`'s roots are laid in, in a `window` as the stylesheet
/// lays it ([`SkinTree::boxes`]): the window less the tree's outsets.
fn pack_box(tree: &SkinTree, (w, h): (i32, i32)) -> [i32; 4] {
    let o = tree.outsets;
    let (left, top) = (i32::from(o.left), i32::from(o.top));
    [
        left,
        top,
        w - left - i32::from(o.right),
        h - top - i32::from(o.bottom),
    ]
}

/// The served target boxes of `tree` (laid in `pack`, its outsets taken
/// off) that cover the centre of another listening node's art box shown
/// with it: a press there would land on the covering node (its target box
/// over its neighbour's art) or on a node that never listens.
fn covered(tree: &SkinTree, pack: [i32; 4]) -> Vec<String> {
    let apart = validate::Apart::new(&tree.root);
    let mut all = Vec::new();
    laid_all(&tree.root, pack, &[], &mut all);
    // A target box on a node that never listens catches the press and
    // drops it (the real-app Friends tab: `TabNormalText` over the
    // `TabNormalButton` part that carries the press).
    let mut out: Vec<String> = all
        .iter()
        .filter(|(n, _, _)| n.target.is_some() && !shell_listens(n))
        .map(|(n, b, _)| format!("{} {b:?} serves a target and never listens", n.id))
        .collect();
    let at = |b: [i32; 4], (cx, cy): (i32, i32)| {
        2 * b[0] <= cx && cx < 2 * (b[0] + b[2]) && 2 * b[1] <= cy && cy < 2 * (b[1] + b[3])
    };
    let inside = |a: [i32; 4], b: [i32; 4]| {
        b[0] <= a[0] && a[0] + a[2] <= b[0] + b[2] && b[1] <= a[1] && a[1] + a[3] <= b[1] + b[3]
    };
    // A grip's served box lies under the nodes it yields to (plan SK11),
    // so their press reaches them; a node whose
    // art holds `m`'s art is `m`'s backdrop, under it; where `m`'s art
    // already holds the centre, the pack's stacking decides (rule 4).
    let stack = validate::Stack::new(&tree.root);
    let listening = all
        .iter()
        .filter(|(n, b, _)| shell_listens(n) && n.resize_edge().is_none() && b[2] > 0 && b[3] > 0);
    for (l, lb, up) in listening {
        let c = (2 * lb[0] + lb[2], 2 * lb[1] + lb[3]);
        for (m, mb, _) in &all {
            let Some(t) = m.target else { continue };
            let skip = m.id == l.id
                || up.contains(&m.id.as_str())
                || apart.apart(&l.id, &m.id)
                || inside(*mb, *lb)
                || at(*mb, c)
                || (m.resize_edge().is_some() && stack.yields(&m.id, &l.id));
            if skip {
                continue;
            }
            let g = target::outer(*mb, t);
            if at(g, c) {
                out.push(format!("{} {g:?} covers {} {lb:?}", m.id, l.id));
            }
        }
    }
    out
}

/// Plan SK9 (the real-app regression after SK7: a click at the centre of
/// the shown Friends tab of seven packs landed on no listening node): at
/// the centre of every listening node's art box in the eleven built-ins'
/// trees and popups, the only target box is its own or an ancestor's
/// (which it paints over), so the press reaches it.
#[test]
fn no_target_box_covers_another_listening_nodes_centre() {
    let mut found = Vec::new();
    for b in builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let m = manifest::served(&package).unwrap();
        for (window, l) in &m.windows {
            let mut trees = Vec::new();
            if let Some(t) = &l.tree {
                trees.push((String::new(), t, pack_box(t, target::window(l))));
            }
            for (name, p) in &l.popups {
                let at = target::window(&super::popup::frame(p, l.min_height));
                trees.push((format!(" popup {name}"), &p.tree, pack_box(&p.tree, at)));
            }
            for (popup, tree, at) in trees {
                for c in covered(tree, at) {
                    found.push(format!("{} {window:?}{popup}: {c}", b.id));
                }
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// Plan SK11: every resize grip of the eleven built-ins is a pointer
/// target: under 24 px it is served a target box, which never lies over the
/// target box of a listening node it stacks above (it may lie under one,
/// which then keeps its press); SpaceCowboy's frame grips (the real app's
/// list) each serve one, the corners 24 x 24.
#[test]
fn every_builtin_grip_is_a_served_target() {
    let mut cowboy = Vec::new();
    for b in builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let m = manifest::served(&package).unwrap();
        for (window, l) in &m.windows {
            let mut trees = Vec::new();
            if let Some(t) = &l.tree {
                trees.push((String::new(), t, target::window(l)));
            }
            for (name, p) in &l.popups {
                let at = target::window(&super::popup::frame(p, l.min_height));
                trees.push((format!(" popup {name}"), &p.tree, at));
            }
            for (popup, tree, at) in trees {
                let laid = target::targets(tree, at);
                let stack = validate::Stack::new(&tree.root);
                let apart = validate::Apart::new(&tree.root);
                for g in target::grip_targets(tree, at, &laid) {
                    // The box holds the art and grows it (or keeps it at 24 px).
                    let (a, r) = (g.art, g.area);
                    let holds = r[0] <= a[0]
                        && r[1] <= a[1]
                        && a[0] + a[2] <= r[0] + r[2]
                        && a[1] + a[3] <= r[1] + r[3];
                    assert!(holds, "{} {}: {r:?} drops its art {a:?}", b.id, g.id);
                    for u in laid.iter().filter(|u| !apart.apart(&g.id, &u.id)) {
                        // Above `u`, the grip's box meets `u`'s only on the
                        // grip's own art (the pack's layout, rule 4).
                        let (r, o) = (g.area, u.area);
                        let meet = [
                            r[0].max(o[0]),
                            r[1].max(o[1]),
                            (r[0] + r[2]).min(o[0] + o[2]),
                            (r[1] + r[3]).min(o[1] + o[3]),
                        ];
                        let a = g.art;
                        let on_art = meet[0] >= a[0]
                            && meet[1] >= a[1]
                            && meet[2] <= a[0] + a[2]
                            && meet[3] <= a[1] + a[3];
                        let over =
                            validate::overlap(r, o) && !stack.yields(&g.id, &u.id) && !on_art;
                        assert!(
                            !over,
                            "{} {window:?}{popup}: grip {} {r:?} over {} {o:?}",
                            b.id, g.id, u.id
                        );
                    }
                    if b.id == "spacecowboy" && popup.is_empty() {
                        cowboy.push((g.id.clone(), g.area, g.rule));
                    }
                }
            }
        }
    }
    println!("SPACECOWBOY {cowboy:?}");
}
