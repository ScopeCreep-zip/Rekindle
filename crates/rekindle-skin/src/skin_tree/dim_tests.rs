//! The served Disabled copies (plan E7.7 CR1 / DIS3).

use std::collections::HashMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionState, SkinManifest, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{DisabledBy, NodePieces, SkinNode, SkinTree};

use super::super::disabled::{shell_only, visible_difference, Measured, VISIBLE_PIXELS};
use super::super::{popup, target};
use super::{crop, Made, Under};
use crate::package::{self, Package};
use crate::{builtin, manifest};

fn pack(id: &str) -> Package<'static> {
    let (_, b) = builtin::find(&format!("builtin:{id}")).unwrap();
    package::read(b.bytes).unwrap()
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

/// The first frame of `name`, from the made images or the package.
fn frame(name: &str, made: &Made, p: &Package<'_>) -> RgbaImage {
    let bytes = made.get(name).map_or_else(|| p.get(name).unwrap(), Vec::as_slice);
    crate::regions::frames(name, bytes)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

/// The visibly changed pixels from `from` to `to`, slot by slot.
fn changed(from: &NodePieces, to: &NodePieces, made: &Made, p: &Package<'_>) -> u64 {
    assert_eq!(from.keys().collect::<Vec<_>>(), to.keys().collect::<Vec<_>>());
    from.iter()
        .map(|(slot, a)| {
            visible_difference(&frame(&a.image, made, p), &frame(&to[slot].image, made, p))
        })
        .sum()
}

/// Each tree of `m` laid in its window at its served minimum, named.
fn trees(m: &SkinManifest) -> Vec<(String, &SkinTree, (i32, i32))> {
    let mut out = Vec::new();
    for (window, l) in &m.windows {
        if let Some(t) = &l.tree {
            out.push((format!("{window:?}"), t, target::window(l)));
        }
        for (name, pop) in &l.popups {
            let at = target::window(&popup::frame(pop, l.min_height));
            out.push((format!("{window:?}/{name}"), &pop.tree, at));
        }
    }
    out
}

/// What the node shows before it is disabled: its own Normal set, else its
/// nearest ancestor's art cropped to its box (decoded into `measured`).
fn before(
    n: &SkinNode,
    at: [i32; 4],
    under: Option<&Under>,
    p: &Package<'_>,
    measured: &mut Measured,
) -> NodePieces {
    match n.art.as_ref().filter(|a| !a.normal.is_empty()) {
        Some(a) => a.normal.clone(),
        None => crop(under.unwrap(), at, p, measured).unwrap().unwrap(),
    }
}

/// Every interactive node of `trees` with its box, the art under it, and
/// its tree's name.
fn walk<'a>(
    nodes: &'a [SkinNode],
    under: Option<&Under>,
    boxes: &HashMap<&str, [i32; 4]>,
    f: &mut impl FnMut(&'a SkinNode, [i32; 4], Option<&Under>),
) {
    for n in nodes {
        let at = boxes[n.id.as_str()];
        if n.interactive() {
            f(n, at, under);
        }
        let own = n
            .art
            .as_ref()
            .filter(|a| !a.normal.is_empty())
            .map(|a| Under {
                set: a.normal.clone(),
                at,
            });
        walk(&n.children, own.as_ref().or(under), boxes, f);
    }
}

fn served(id: &str) -> (SkinManifest, Made) {
    manifest::served_made(&pack(id)).unwrap()
}

fn popup_tree<'a>(m: &'a SkinManifest, window: SkinWindow, name: &str) -> &'a SkinTree {
    &m.windows[&window].popups[name].tree
}

/// The served Disabled set of `n`, a copy the serve made.
fn copy_of(n: &SkinNode) -> &NodePieces {
    assert_eq!(n.disabled_by, Some(DisabledBy::Served), "{}", n.id);
    let set = &n.art.as_ref().unwrap().states[&RegionState::Disabled];
    assert!(set.values().all(|p| p.image.starts_with(super::MADE)));
    set
}

/// Alloy's info-popup `HostChatButton`: opaque Normal art identical to its
/// `JoinChatTab` drawn under it, so the shell's opacity changed 0 px; its
/// served copy changes at least the bar.
#[test]
fn alloy_host_chat_button_serves_a_visibly_dimmed_copy() {
    let (m, made) = served("alloy");
    let tree = popup_tree(&m, SkinWindow::Main, "Infoskin");
    let n = find(&tree.root, "HostChatButton").unwrap();
    let normal = &n.art.as_ref().unwrap().normal;
    let tab = &find(&tree.root, "JoinChatTab").unwrap().art.as_ref().unwrap().normal;
    let p = pack("alloy");
    assert_eq!(changed(normal, tab, &made, &p), 0, "the art equals the tab's");
    assert!(changed(normal, copy_of(n), &made, &p) >= VISIBLE_PIXELS);
}

/// Combat's `JoinButton@Friends`: the same, over `FriendsButtons.JoinPlayTab`.
#[test]
fn combat_join_button_serves_a_visibly_dimmed_copy() {
    let (m, made) = served("combat");
    let tree = m.windows[&SkinWindow::Main].tree.as_ref().unwrap();
    let n = find(&tree.root, "JoinButton@Friends").unwrap();
    let normal = &n.art.as_ref().unwrap().normal;
    let tab = find(&tree.root, "FriendsButtons.JoinPlayTab").unwrap();
    let p = pack("combat");
    assert_eq!(changed(normal, &tab.art.as_ref().unwrap().normal, &made, &p), 0);
    assert!(changed(normal, copy_of(n), &made, &p) >= VISIBLE_PIXELS);
}

/// Alloy's art-less `JoinChatButton`: its tab alone draws what is seen, so
/// its copy is the tab's art cropped to its box and dimmed by one filter.
#[test]
fn alloy_join_chat_button_serves_the_tabs_region_cropped_and_dimmed() {
    let (m, made) = served("alloy");
    let l: &WindowLayout = &m.windows[&SkinWindow::Main];
    let pop = &l.popups["Infoskin"];
    let (w, h) = target::window(&popup::frame(pop, l.min_height));
    let boxes: HashMap<&str, [i32; 4]> = pop
        .tree
        .boxes(w, h)
        .into_iter()
        .map(|(n, b)| (n.id.as_str(), b))
        .collect();
    let n = find(&pop.tree.root, "JoinChatButton").unwrap();
    assert!(n.art.as_ref().unwrap().normal.is_empty(), "no art of its own");
    let set = copy_of(n);
    let piece = &set[&rekindle_types::dto::skin_tree::PieceSlot::Middle];
    let at = boxes["JoinChatButton"];
    assert_eq!([i32::from(piece.width), i32::from(piece.height)], [at[2], at[3]]);
    let tab = boxes["JoinChatTab"];
    let p = pack("alloy");
    let tab_art = frame(
        &find(&pop.tree.root, "JoinChatTab").unwrap().art.as_ref().unwrap().normal
            [&rekindle_types::dto::skin_tree::PieceSlot::Middle]
            .image,
        &made,
        &p,
    );
    let copy = frame(&piece.image, &made, &p);
    let (dx, dy) = (
        u32::try_from(at[0] - tab[0]).unwrap(),
        u32::try_from(at[1] - tab[1]).unwrap(),
    );
    let mut region = RgbaImage::new(copy.width(), copy.height());
    for (x, y, px) in region.enumerate_pixels_mut() {
        *px = *tab_art.get_pixel(x + dx, y + dy);
    }
    assert!(visible_difference(&region, &copy) >= VISIBLE_PIXELS);
    let dimmed_by = |dim: super::Dim| {
        region
            .pixels()
            .zip(copy.pixels())
            .all(|(a, b)| (0..3).all(|c| dim.channel(a.0[c]) == b.0[c]) && a.0[3] == b.0[3])
    };
    assert!(super::Dim::ALL.into_iter().any(dimmed_by), "a dimmed crop");
}

/// Every interactive node of every built-in pack is visibly disabled by
/// whoever draws it: a served copy changes at least the bar against what
/// the node shows before; a node the shell still dims carries what a copy
/// cannot ([`shell_only`]) or has nothing painted at its box.
#[test]
fn every_builtin_interactive_node_is_visibly_disabled() {
    let mut weak = Vec::new();
    let mut shell = Vec::new();
    let mut served_nodes = 0;
    for b in builtin::PACKS {
        let p = package::read(b.bytes).unwrap();
        let (m, made) = manifest::served_made(&p).unwrap();
        let mut measured = Measured::default();
        for (at_name, tree, (w, h)) in trees(&m) {
            let boxes: HashMap<&str, [i32; 4]> = tree
                .boxes(w, h)
                .into_iter()
                .map(|(n, b)| (n.id.as_str(), b))
                .collect();
            walk(&tree.root, None, &boxes, &mut |n, at, under| {
                match n.disabled_by {
                    Some(DisabledBy::Served) => {
                        served_nodes += 1;
                        let from = before(n, at, under, &p, &mut measured);
                        let mut both = made.clone();
                        for piece in from.values() {
                            if !piece.image.starts_with(super::MADE) {
                                continue;
                            }
                            if let Some(f) = measured.frames.get(&piece.image) {
                                let png = rekindle_image::encode_png(
                                    &rekindle_image::DynamicImage::ImageRgba8(f.clone()),
                                )
                                .unwrap();
                                both.entry(piece.image.clone()).or_insert(png);
                            }
                        }
                        let c = changed(&from, copy_of(n), &both, &p);
                        if c < VISIBLE_PIXELS {
                            weak.push(format!("{} {at_name} {} {c}", b.id, n.id));
                        }
                    }
                    Some(DisabledBy::Shell) => {
                        // Left to the shell only what a copy cannot carry,
                        // or a box with nothing painted at it.
                        let painted = n.art.as_ref().is_some_and(|a| !a.normal.is_empty())
                            || under.is_some_and(|u| {
                                let c = crop(u, at, &p, &mut measured).unwrap();
                                c.is_some_and(|c| {
                                    c.values().any(|piece| {
                                        measured.frames[&piece.image].pixels().any(|px| px.0[3] > 0)
                                    })
                                })
                            });
                        if painted && !shell_only(n) {
                            shell.push(format!("{} {at_name} {}", b.id, n.id));
                        }
                    }
                    Some(DisabledBy::Pack) => {}
                    None => weak.push(format!("{} {at_name} {} unmarked", b.id, n.id)),
                }
            });
        }
    }
    assert!(weak.is_empty(), "under the bar: {weak:#?}");
    assert!(served_nodes > 0);
    shell.sort();
    shell.dedup();
    assert!(shell.is_empty(), "painted, yet the shell's: {shell:#?}");
}

/// xSLIMFIRE's and Symbiosis's own visibly distinct Disabled art stays as
/// the pack drew it (DIS2), never replaced by a copy.
#[test]
fn a_packs_own_disabled_art_is_unchanged() {
    for id in ["xslimfire", "symbiosis"] {
        let p = pack(id);
        let authored = manifest::parse(&p).unwrap();
        let (m, _) = served(id);
        let authored: HashMap<String, &SkinTree> = trees(&authored)
            .into_iter()
            .map(|(name, t, _)| (name, t))
            .collect();
        let mut packs = 0;
        for (name, tree, _) in trees(&m) {
            let Some(authored_tree) = authored.get(&name) else {
                continue;
            };
            let mut nodes = Vec::new();
            collect(&tree.root, &mut nodes);
            for n in nodes {
                if n.disabled_by != Some(DisabledBy::Pack) {
                    continue;
                }
                packs += 1;
                let Some(art) = &n.art else { continue };
                // A node the placements add (the Communities clone) has no
                // authored counterpart.
                let Some(o) = find(&authored_tree.root, &n.id) else {
                    continue;
                };
                let original = o.art.as_ref().and_then(|a| a.states.get(&RegionState::Disabled));
                assert_eq!(art.states.get(&RegionState::Disabled), original, "{id} {}", n.id);
            }
        }
        assert!(packs > 0, "{id} has Pack nodes");
    }
}

fn collect<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a SkinNode>) {
    for n in nodes {
        out.push(n);
        collect(&n.children, out);
    }
}
