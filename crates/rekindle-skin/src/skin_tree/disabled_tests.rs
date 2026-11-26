//! The served `disabledBy` of each interactive node (plan E7.7 CR1).

use rekindle_types::dto::skin::RegionState;
use rekindle_types::dto::skin_tree::{DisabledBy, NodeRole, SkinNode};

use super::disabled::pack_art;
use super::golden_tests::main_layout;
use crate::{builtin, manifest, package};

fn find<'a>(nodes: &'a [SkinNode], id: &str) -> Option<&'a SkinNode> {
    nodes.iter().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            find(&n.children, id)
        }
    })
}

fn each(nodes: &[SkinNode], f: &mut impl FnMut(&SkinNode)) {
    for n in nodes {
        f(n);
        each(&n.children, f);
    }
}

#[test]
fn xslimfire_chat_button_is_disabled_by_its_packs_art() {
    let tree = main_layout("xslimfire").tree.unwrap();
    let chat = find(&tree.root, "ChatButton").unwrap();
    assert_eq!(chat.disabled_by, Some(DisabledBy::Pack));
    assert_eq!(serde_json::to_value(chat).unwrap()["disabledBy"], "pack");
}

#[test]
fn the_composed_communities_tab_is_disabled_by_the_shell() {
    let tree = main_layout("xslimfire").tree.unwrap();
    let tab = find(&tree.root, "TabDownloadButton.Communities").unwrap();
    assert_eq!(tab.disabled_by, Some(DisabledBy::Shell));
    assert_eq!(serde_json::to_value(tab).unwrap()["disabledBy"], "shell");
}

#[test]
fn every_interactive_node_and_only_those_say_who_disables_it() {
    for id in ["xslimfire", "combat"] {
        let tree = main_layout(id).tree.unwrap();
        each(&tree.root, &mut |n| {
            assert_eq!(n.disabled_by.is_some(), n.interactive(), "{id} {}", n.id);
        });
    }
}

/// xslimfire's ChatButton with no parts: its own art alone decides.
fn chat_art_only() -> SkinNode {
    let tree = main_layout("xslimfire").tree.unwrap();
    let mut chat = find(&tree.root, "ChatButton").unwrap().clone();
    chat.children.clear();
    chat
}

#[test]
fn a_disabled_look_distinct_from_normal_is_the_packs() {
    let chat = chat_art_only();
    let art = chat.art.as_ref().unwrap();
    assert_ne!(art.states.get(&RegionState::Disabled), Some(&art.normal));
    assert!(art.states.contains_key(&RegionState::Disabled));
    assert!(pack_art(&chat));
}

#[test]
fn a_disabled_look_equal_to_normal_is_the_shells_to_dim() {
    let mut chat = chat_art_only();
    let art = chat.art.as_mut().unwrap();
    art.states.insert(RegionState::Disabled, art.normal.clone());
    assert!(!pack_art(&chat));
}

#[test]
fn no_disabled_look_is_the_shells_to_dim() {
    let mut chat = chat_art_only();
    chat.art
        .as_mut()
        .unwrap()
        .states
        .remove(&RegionState::Disabled);
    assert!(!pack_art(&chat));
}

/// The rule before CR1b: any Disabled state at all, equal to Normal or not.
fn had_disabled_state(n: &SkinNode) -> bool {
    let has = |c: &SkinNode| {
        c.art
            .as_ref()
            .is_some_and(|a| a.states.contains_key(&RegionState::Disabled))
    };
    n.label.is_none()
        && (has(n)
            || n.children
                .iter()
                .any(|c| c.role == NodeRole::Part && has(c)))
}

/// Plan CR1b: every built-in node whose Disabled art equals its Normal art
/// moves from the pack's look to the shell's dimming; the full list is
/// pinned so the effect is reviewed, never silent. Swept 2026-11-25: no
/// built-in pack ships a Disabled set equal to its Normal set, so the list
/// is empty; Combat ships no Disabled state at all, so every interactive
/// Combat node was and stays the shell's to dim.
#[test]
fn builtin_nodes_whose_disabled_art_is_their_normal_art_are_dimmed_by_the_shell() {
    let mut moved = Vec::new();
    let mut combat_pack = Vec::new();
    for b in builtin::PACKS {
        let p = package::read(b.bytes).unwrap();
        let m = manifest::served(&p).unwrap();
        for (window, l) in &m.windows {
            let trees = l.tree.iter().map(|t| (format!("{window:?}"), t)).chain(
                l.popups
                    .iter()
                    .map(|(k, p)| (format!("{window:?}/{k}"), &p.tree)),
            );
            for (at, tree) in trees {
                each(&tree.root, &mut |n| {
                    if n.interactive()
                        && had_disabled_state(n)
                        && n.disabled_by == Some(DisabledBy::Shell)
                    {
                        moved.push(format!("{} {at} {}", b.id, n.id));
                    }
                    if b.id == "combat" && n.disabled_by == Some(DisabledBy::Pack) {
                        combat_pack.push(format!("{at} {}", n.id));
                    }
                });
            }
        }
    }
    moved.sort();
    moved.dedup();
    assert!(moved.is_empty(), "Pack -> Shell: {moved:#?}");
    assert!(
        combat_pack.is_empty(),
        "combat Pack nodes: {combat_pack:#?}"
    );
}
