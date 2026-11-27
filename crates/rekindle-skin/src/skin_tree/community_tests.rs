//! Plan SK17-6 (SKW2, the lead's review of SKW1's real-app community
//! shots): the served Community window's content slot draws above the art
//! it meets (Alloy, Simple X), its title is the pack's text point
//! (xSLIMFIRE, Alloy), and a window composed from a buddy list (Total
//! Chaos) draws none of the buddy list's own nodes and gives the community
//! content the whole content area.

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, SlotKey};

use super::compose;
use crate::{builtin, manifest, package};

/// The large window the slot lift also compares boxes in
/// (`chat_slots::LARGE`).
const LARGE: (i32, i32) = (1200, 900);

/// The built-in's served Community tree and its window at the served
/// minimum.
fn community(id: &str) -> (SkinTree, (i32, i32)) {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    let p = package::read(b.bytes).unwrap();
    let m = manifest::served(&p).unwrap();
    let l = m.windows[&SkinWindow::Community].clone();
    let at = super::target::window(&l);
    (l.tree.unwrap(), at)
}

/// Every node, depth first (the order of `SkinTree::boxes`), with its
/// parent's index.
fn flat<'a>(
    nodes: &'a [SkinNode],
    parent: Option<usize>,
    out: &mut Vec<(&'a SkinNode, Option<usize>)>,
) {
    for n in nodes {
        let i = out.len();
        out.push((n, parent));
        flat(&n.children, Some(i), out);
    }
}

fn under(all: &[(&SkinNode, Option<usize>)], mut k: usize, j: usize) -> bool {
    loop {
        if k == j {
            return true;
        }
        match all[k].1 {
            Some(p) => k = p,
            None => return false,
        }
    }
}

#[test]
fn the_community_content_slots_draw_above_the_art_they_meet() {
    for id in ["alloy", "simple-x"] {
        let (tree, at) = community(id);
        let mut all = Vec::new();
        flat(&tree.root, None, &mut all);
        for key in [SlotKey::ChatArea, SlotKey::ChatEdit] {
            let s = all
                .iter()
                .position(|(n, _)| n.role == NodeRole::Slot(key))
                .unwrap_or_else(|| panic!("{id}: no {key:?} slot"));
            for (w, h) in [at, LARGE] {
                let boxes: Vec<[i32; 4]> = tree.boxes(w, h).into_iter().map(|(_, b)| b).collect();
                assert!(
                    boxes[s][2] > 0 && boxes[s][3] > 0,
                    "{id}: {key:?} has room {:?}",
                    boxes[s]
                );
                // The slot and each group holding it stack above every
                // sibling whose art meets the slot.
                let mut a = s;
                loop {
                    for j in (0..all.len()).filter(|&j| j != a && all[j].1 == all[a].1) {
                        let meets = (0..all.len()).any(|k| {
                            under(&all, k, j)
                                && all[k].0.art.is_some()
                                && super::validate::overlap(boxes[s], boxes[k])
                        });
                        if meets {
                            assert!(
                                all[a].0.z > all[j].0.z,
                                "{id}: {} (z {}) under {} (z {}) whose art meets {key:?}",
                                all[a].0.id,
                                all[a].0.z,
                                all[j].0.id,
                                all[j].0.z
                            );
                        }
                    }
                    match all[a].1 {
                        Some(p) => a = p,
                        None => break,
                    }
                }
            }
        }
    }
}

#[test]
fn the_community_title_is_the_packs_text_point() {
    // The field the pack lays its title in, which `chat_slots::title_leaf`
    // makes a group around the title's text leaf: xSLIMFIRE's text sits in
    // its field (`goldens/chat-xslimfire.txt:3-4`), Alloy's in the field's
    // background part `UserNameBG` (`goldens/chat-alloy.txt:3-5`).
    for (id, want) in [
        ("xslimfire", "ChatUserNameField"),
        ("alloy", "UserNameField"),
    ] {
        let (tree, _) = community(id);
        let mut all = Vec::new();
        flat(&tree.root, None, &mut all);
        let titles: Vec<_> = all
            .iter()
            .filter(|(n, _)| n.role == NodeRole::ChatTitle)
            .collect();
        assert_eq!(titles.len(), 1, "{id}");
        let (t, parent) = titles[0];
        assert!(
            t.art.is_none() && t.children.is_empty(),
            "{id}: {} is a text leaf",
            t.id
        );
        let mut a = *parent;
        let field = loop {
            let i = a.unwrap_or_else(|| panic!("{id}: no group holds {}", t.id));
            if all[i].0.role == NodeRole::Group {
                break all[i].0;
            }
            a = all[i].1;
        };
        assert_eq!(
            field.id, want,
            "{id}: the pack's title field groups the title"
        );
    }
}

#[test]
fn a_community_window_composed_from_a_buddy_list_draws_none_of_its_own_nodes() {
    let (tree, at) = community("total-chaos");
    let mut all = Vec::new();
    flat(&tree.root, None, &mut all);
    let kept: Vec<_> = all
        .iter()
        .filter(|(n, _)| compose::buddy_list_only(n.role))
        .map(|(n, _)| (&n.id, n.role))
        .collect();
    assert!(kept.is_empty(), "buddy-list-only nodes: {kept:?}");
    let only = |key: SlotKey| -> usize {
        let at: Vec<usize> = (0..all.len())
            .filter(|&i| all[i].0.role == NodeRole::Slot(key))
            .collect();
        assert_eq!(at.len(), 1, "one {key:?} slot");
        at[0]
    };
    let (s, e) = (only(SlotKey::ChatArea), only(SlotKey::ChatEdit));
    let p = all[s].1.unwrap();
    assert_eq!(all[p].0.id, "Content", "the slot is the content group's");
    assert_eq!(all[e].1, Some(p), "the composer is the content group's");
    let roles: Vec<NodeRole> = all[p].0.children.iter().map(|c| c.role).collect();
    assert_eq!(
        roles,
        [
            NodeRole::Slot(SlotKey::ChatArea),
            NodeRole::Slot(SlotKey::ChatEdit)
        ],
        "the conversation over the composer alone fill the content group"
    );
    for (w, h) in [at, LARGE] {
        let boxes: Vec<[i32; 4]> = tree.boxes(w, h).into_iter().map(|(_, b)| b).collect();
        let (area, edit, group) = (boxes[s], boxes[e], boxes[p]);
        for b in [area, edit] {
            assert_eq!((b[0], b[2]), (group[0], group[2]), "full width at {w}x{h}");
            assert!(b[3] > 0, "room at {w}x{h}: {b:?}");
        }
        assert_eq!(
            area[1], group[1],
            "the conversation starts at the top at {w}x{h}"
        );
        assert_eq!(
            area[1] + area[3],
            edit[1],
            "the composer is under it at {w}x{h}"
        );
        assert_eq!(
            edit[1] + edit[3],
            group[1] + group[3],
            "the composer ends at the bottom at {w}x{h}"
        );
        assert_eq!(edit[3], i32::from(compose::COMPOSER_HEIGHT), "{w}x{h}");
    }
    // The frame, title strip, window buttons and menus stay.
    for role in [NodeRole::Drag, NodeRole::MenuBar, NodeRole::MenuTitle] {
        assert!(all.iter().any(|(n, _)| n.role == role), "{role:?} kept");
    }
}

/// Defect 3 (SKW2): the served content slot reaches the composer line, so
/// a conversation that stops short is the frontend's, never the tree's. The
/// slot ends at most `REACH` px above the first composer-line slot under it
/// (the typing notice or the composer): Symbiosis's typing notice is 2 px
/// under its list, xSLIMFIRE's composer 18 px (its pack's own divider art),
/// Total Chaos's composer flush (`compose::content`).
#[test]
fn the_community_content_slot_reaches_the_composer() {
    const REACH: i32 = 20;
    for id in ["symbiosis", "xslimfire", "total-chaos"] {
        let (tree, at) = community(id);
        for (w, h) in [at, (900, 650), LARGE] {
            let boxes = tree.boxes(w, h);
            let of = |key: SlotKey| -> Vec<[i32; 4]> {
                boxes
                    .iter()
                    .filter(|(n, _)| n.role == NodeRole::Slot(key))
                    .map(|(_, b)| *b)
                    .collect()
            };
            let (area, edit) = (of(SlotKey::ChatArea), of(SlotKey::ChatEdit));
            assert_eq!(area.len(), 1, "{id}");
            assert_eq!(edit.len(), 1, "{id}");
            let end = area[0][1] + area[0][3];
            let line = edit
                .iter()
                .chain(of(SlotKey::TypingNotice).iter())
                .map(|b| b[1])
                .filter(|&top| top >= end)
                .min()
                .unwrap_or_else(|| panic!("{id}: the list ends above the composer at {w}x{h}"));
            assert!(
                line - end <= REACH,
                "{id}: the list ends {} px above the composer line at {w}x{h}",
                line - end
            );
        }
    }
}

/// Plan SKW3: xfire-classic's served Community window, composed from its
/// group chat (`xfiregroupchatwindow.xml`, whose `ChatEdit` keeps its end
/// inset past the divider), against its golden.
#[test]
fn xfire_classics_community_tree_matches_its_golden() {
    let (tree, _) = community("xfire-classic");
    let mut got = String::new();
    super::golden_tests::lines(&tree.root, 0, &mut got);
    assert_eq!(
        got,
        include_str!("goldens/community-xfire-classic.txt"),
        "=== community-xfire-classic begin\n{got}=== community-xfire-classic end"
    );
}
