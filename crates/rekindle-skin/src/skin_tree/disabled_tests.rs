//! The served `disabledBy` of each interactive node (plan E7.7 CR1).

use rekindle_types::dto::skin::RegionState;
use rekindle_types::dto::skin_tree::{DisabledBy, NodeRole, SkinNode};

use rekindle_image::{Rgba, RgbaImage};

use super::disabled::{pack_art, visible_difference, Measured, VISIBLE_PIXELS};
use super::golden_tests::main_layout;
use crate::package::Package;
use crate::{builtin, manifest, package};

fn pack(id: &str) -> Package<'static> {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    package::read(b.bytes).unwrap()
}

fn is_pack_art(id: &str, n: &SkinNode) -> bool {
    pack_art(n, &pack(id), &mut Measured::default()).unwrap()
}

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

/// Total Chaos's Disabled `CommunitiesOpen` differs from its Normal art by
/// a few pixels: the refused button would look pressable, so the shell
/// dims it (served pieces: 658 of 1710 pixels differ at all, none by
/// more than the channel delta; real app: 12 px changed, under the 20 px bar).
#[test]
fn total_chaos_communities_open_is_disabled_by_a_served_copy() {
    let tree = main_layout("total-chaos").tree.unwrap();
    let open = find(&tree.root, "CommunitiesOpen").unwrap();
    let art = open.art.as_ref().unwrap();
    assert!(art.states.contains_key(&RegionState::Disabled));
    assert_ne!(art.states.get(&RegionState::Disabled), Some(&art.normal));
    // DIS3: the near-copy is replaced by the serve's visibly dimmed copy.
    assert_eq!(open.disabled_by, Some(DisabledBy::Served));
}

fn image(w: u32, h: u32, p: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(p))
}

#[test]
fn the_measure_counts_only_visible_changes() {
    let normal = image(10, 10, [200, 180, 160, 255]);
    assert_eq!(visible_difference(&normal, &normal.clone()), 0);
    let mut few = normal.clone();
    for x in 0..5 {
        few.put_pixel(x, 0, Rgba([0, 0, 0, 255]));
    }
    assert_eq!(visible_difference(&normal, &few), 5);
    assert!(visible_difference(&normal, &few) < VISIBLE_PIXELS);
    let mut faint = normal.clone();
    for p in faint.pixels_mut() {
        p.0[0] -= 20;
    }
    assert_eq!(visible_difference(&normal, &faint), 0);
    let mut dimmed = normal.clone();
    for p in dimmed.pixels_mut() {
        for c in &mut p.0[..3] {
            *c /= 2;
        }
    }
    assert_eq!(visible_difference(&normal, &dimmed), 100);
    assert!(visible_difference(&normal, &dimmed) >= VISIBLE_PIXELS);
    assert_eq!(visible_difference(&normal, &image(5, 4, [0; 4])), 100);
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
    assert!(is_pack_art("xslimfire", &chat));
}

#[test]
fn a_disabled_look_equal_to_normal_is_the_shells_to_dim() {
    let mut chat = chat_art_only();
    let art = chat.art.as_mut().unwrap();
    art.states.insert(RegionState::Disabled, art.normal.clone());
    assert!(!is_pack_art("xslimfire", &chat));
}

#[test]
fn no_disabled_look_is_the_shells_to_dim() {
    let mut chat = chat_art_only();
    chat.art
        .as_mut()
        .unwrap()
        .states
        .remove(&RegionState::Disabled);
    assert!(!is_pack_art("xslimfire", &chat));
}

/// The rule before CR1b: any Disabled state the pack ships at all, equal
/// to Normal or not (a copy the serve made, DIS3, is not the pack's).
fn had_disabled_state(n: &SkinNode) -> bool {
    let has = |c: &SkinNode| {
        c.art.as_ref().is_some_and(|a| {
            a.states.get(&RegionState::Disabled).is_some_and(|s| {
                s.values()
                    .all(|p| !p.image.starts_with(super::dim::MADE))
            })
        })
    };
    n.label.is_none()
        && (has(n)
            || n.children
                .iter()
                .any(|c| c.role == NodeRole::Part && has(c)))
}

/// Plan CR1b and DIS2: every built-in node that ships a Disabled state but
/// moves from the pack's look to the shell's dimming, because its Disabled
/// art is its Normal art or differs from it by fewer than
/// [`VISIBLE_PIXELS`] visibly changed pixels; the full list is pinned so the
/// effect is reviewed, never silent. Swept 2026-11-26: only Total Chaos's
/// Disabled pieces are near-copies of their Normal ones (at most 4 pixels
/// change by more than the channel delta; every node that stays the pack's
/// changes 24 or more); Combat ships no Disabled state at all, so every
/// interactive Combat node was and stays the shell's to dim.
#[test]
fn builtin_nodes_whose_disabled_art_is_not_visibly_distinct_are_dimmed_by_the_shell() {
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
                        && matches!(
                            n.disabled_by,
                            Some(DisabledBy::Served | DisabledBy::Shell)
                        )
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
    let expected = [
        "total-chaos Community/InfoPopup InfoJoinButton",
        "total-chaos Main AddServerButton",
        "total-chaos Main CommunitiesNew",
        "total-chaos Main CommunitiesOpen",
        "total-chaos Main DownloadInfoButton",
        "total-chaos Main ExecuteDownloadButton",
        "total-chaos Main ExpandButton",
        "total-chaos Main ImButton",
        "total-chaos Main InfoButton",
        "total-chaos Main JoinFriendButton",
        "total-chaos Main JoinServerButton",
        "total-chaos Main RefreshButton",
        "total-chaos Main StartDownloadButton",
        "total-chaos Main StopDownloadButton",
        "total-chaos Main/InfoPopup InfoJoinButton",
    ];
    assert_eq!(moved, expected, "Pack -> Shell: {moved:#?}");
    assert!(
        combat_pack.is_empty(),
        "combat Pack nodes: {combat_pack:#?}"
    );
}
