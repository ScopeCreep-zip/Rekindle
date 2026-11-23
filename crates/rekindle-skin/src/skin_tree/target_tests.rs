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
    css::tree_css(".f", l.tree.as_ref().unwrap(), l, &BTreeMap::new(), "x").unwrap()
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
