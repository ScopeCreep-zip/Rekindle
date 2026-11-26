//! The served Disabled copies (plan E7.7 CR1 / DIS3).

use std::collections::HashMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionState, SkinManifest, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_tree::{
    DisabledBy, NodePiece, NodePieces, PieceSlot, SkinNode, SkinTree,
};

use super::super::disabled::{
    changed_pixels, shell_only, visible_difference, visibly_changed, visibly_differs, Measured,
    VISIBLE_PIXELS,
};
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
    let bytes = made
        .get(name)
        .map_or_else(|| p.get(name).unwrap(), Vec::as_slice);
    crate::regions::frames(name, bytes)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

/// The visibly changed pixels from `from` to `to`, slot by slot.
fn changed(from: &NodePieces, to: &NodePieces, made: &Made, p: &Package<'_>) -> u64 {
    assert_eq!(
        from.keys().collect::<Vec<_>>(),
        to.keys().collect::<Vec<_>>()
    );
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

/// The served refusal set of `n`, a copy the serve made.
fn copy_of(n: &SkinNode) -> &NodePieces {
    assert_eq!(n.disabled_by, Some(DisabledBy::Served), "{}", n.id);
    let set = n.art.as_ref().unwrap().refused.as_ref().unwrap();
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
    let tab = &find(&tree.root, "JoinChatTab")
        .unwrap()
        .art
        .as_ref()
        .unwrap()
        .normal;
    let p = pack("alloy");
    assert_eq!(
        changed(normal, tab, &made, &p),
        0,
        "the art equals the tab's"
    );
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
    assert_eq!(
        changed(normal, &tab.art.as_ref().unwrap().normal, &made, &p),
        0
    );
    assert!(changed(normal, copy_of(n), &made, &p) >= VISIBLE_PIXELS);
}

/// Alloy's art-less `JoinChatButton`: its tab alone draws what is seen, so
/// its copy is the tab's art cropped to its box and dimmed by one filter.
#[test]
fn alloy_join_chat_button_serves_the_tabs_region_cropped_and_dimmed() {
    let (served_manifest, made) = served("alloy");
    let layout: &WindowLayout = &served_manifest.windows[&SkinWindow::Main];
    let pop = &layout.popups["Infoskin"];
    let (win_w, win_h) = target::window(&popup::frame(pop, layout.min_height));
    let boxes: HashMap<&str, [i32; 4]> = pop
        .tree
        .boxes(win_w, win_h)
        .into_iter()
        .map(|(n, b)| (n.id.as_str(), b))
        .collect();
    let n = find(&pop.tree.root, "JoinChatButton").unwrap();
    assert!(
        n.art.as_ref().unwrap().normal.is_empty(),
        "no art of its own"
    );
    let set = copy_of(n);
    let piece = &set[&rekindle_types::dto::skin_tree::PieceSlot::Middle];
    let at = boxes["JoinChatButton"];
    assert_eq!(
        [i32::from(piece.width), i32::from(piece.height)],
        [at[2], at[3]]
    );
    let tab = boxes["JoinChatTab"];
    let pkg = pack("alloy");
    let tab_art = frame(
        &find(&pop.tree.root, "JoinChatTab")
            .unwrap()
            .art
            .as_ref()
            .unwrap()
            .normal[&rekindle_types::dto::skin_tree::PieceSlot::Middle]
            .image,
        &made,
        &pkg,
    );
    let copy = frame(&piece.image, &made, &pkg);
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
/// whoever draws it: a served copy is visibly changed from what the node
/// shows before ([`visibly_changed`]); a node the shell still dims carries what a copy
/// cannot ([`shell_only`]) or has nothing painted at its box.
#[test]
fn every_builtin_interactive_node_is_visibly_disabled() {
    let mut weak = Vec::new();
    let mut shell = Vec::new();
    let mut served_nodes = 0;
    for b in builtin::PACKS {
        let package = package::read(b.bytes).unwrap();
        let (m, made) = manifest::served_made(&package).unwrap();
        let mut measured = Measured::default();
        for (at_name, tree, (win_w, win_h)) in trees(&m) {
            let boxes: HashMap<&str, [i32; 4]> = tree
                .boxes(win_w, win_h)
                .into_iter()
                .map(|(n, b)| (n.id.as_str(), b))
                .collect();
            each(&tree.root, &mut |node| {
                // A copy is the only art the serve adds: a node with none
                // keeps the pack's art, never an empty set of its own.
                let refused = node.art.as_ref().and_then(|art| art.refused.as_ref());
                assert_eq!(
                    refused.is_some(),
                    node.disabled_by == Some(DisabledBy::Served),
                    "{} {at_name} {}",
                    b.id,
                    node.id
                );
                if let Some(art) = node.art.as_ref().filter(|_| refused.is_none()) {
                    let empty = art.normal.is_empty() && art.states.is_empty();
                    assert!(!empty, "{} {at_name} {}: empty art", b.id, node.id);
                }
            });
            walk(&tree.root, None, &boxes, &mut |n, at, under| {
                match n.disabled_by {
                    Some(DisabledBy::Served) => {
                        served_nodes += 1;
                        let from = before(n, at, under, &package, &mut measured);
                        let copy = copy_of(n);
                        // The copy's pixels as served: decoded from its PNG.
                        for piece in copy.values() {
                            measured
                                .frames
                                .entry(piece.image.clone())
                                .or_insert_with(|| frame(&piece.image, &made, &package));
                        }
                        let c = changed_pixels(&from, copy, &package, &mut measured).unwrap();
                        if !visibly_changed(&from, copy, &package, &mut measured).unwrap() {
                            weak.push(format!("{} {at_name} {} {c}", b.id, n.id));
                        }
                    }
                    Some(DisabledBy::Shell) => {
                        // Left to the shell only what a copy cannot carry,
                        // or a box with nothing painted at it.
                        let painted = n.art.as_ref().is_some_and(|a| !a.normal.is_empty())
                            || under.is_some_and(|u| {
                                let c = crop(u, at, &package, &mut measured).unwrap();
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
                let original = o
                    .art
                    .as_ref()
                    .and_then(|a| a.states.get(&RegionState::Disabled));
                assert_eq!(
                    art.states.get(&RegionState::Disabled),
                    original,
                    "{id} {}",
                    n.id
                );
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

fn each<'a>(nodes: &'a [SkinNode], f: &mut impl FnMut(&'a SkinNode)) {
    let mut all = Vec::new();
    collect(nodes, &mut all);
    all.into_iter().for_each(f);
}

/// The served stylesheet's rule heads laid on the node `id` of `pack`'s
/// main tree, and the node.
fn rule_heads(pack_id: &str, id: &str) -> (SkinNode, Vec<String>) {
    let p = pack(pack_id);
    let (mut m, made) = served(pack_id);
    let mut files = crate::images::validate(&p, &m).unwrap();
    files.extend(made);
    let l = m.windows.remove(&SkinWindow::Main).unwrap();
    let tree = l.tree.clone().unwrap();
    let css = super::super::css::tree_css(".f", &tree, &l, &files, "x").unwrap();
    let sel = format!(".f [data-skin-node=\"{id}\"]");
    let heads = css
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.ends_with('{')
                && l.strip_prefix(sel.as_str())
                    .is_some_and(|r| !r.starts_with(' '))
        })
        .map(str::to_owned)
        .collect();
    (find(&tree.root, id).unwrap().clone(), heads)
}

/// Lead ruling 1: the served copy is the refusal's look only. Its rule
/// excludes the Host-sent `windowInactive` cause, so Combat's
/// `JoinButton@Friends` (no pack Disabled art) shows its Normal art while
/// its window is inactive; Total Chaos's `CommunitiesOpen` keeps the
/// pack's own Disabled art for that look beside the copy for a refusal.
#[test]
fn the_served_copy_draws_for_a_refusal_never_for_an_inactive_window() {
    let refused = "[data-state=\"disabled\"]:not([data-state-cause=\"windowInactive\"])";
    let (n, heads) = rule_heads("combat", "JoinButton@Friends");
    assert!(!n
        .art
        .as_ref()
        .unwrap()
        .states
        .contains_key(&RegionState::Disabled));
    copy_of(&n);
    let sel = ".f [data-skin-node=\"JoinButton@Friends\"]";
    assert!(heads.contains(&format!("{sel}{refused} {{")), "{heads:#?}");
    for h in heads
        .iter()
        .filter(|h| h.contains("data-state=\"disabled\""))
    {
        assert!(
            h.contains(":not([data-state-cause=\"windowInactive\"])"),
            "{h}"
        );
    }
    let (n, heads) = rule_heads("total-chaos", "CommunitiesOpen");
    assert!(n
        .art
        .as_ref()
        .unwrap()
        .states
        .contains_key(&RegionState::Disabled));
    copy_of(&n);
    let sel = ".f [data-skin-node=\"CommunitiesOpen\"]";
    assert!(
        heads.contains(&format!("{sel}[data-state=\"disabled\"] {{")),
        "{heads:#?}"
    );
    assert!(heads.contains(&format!("{sel}{refused} {{")), "{heads:#?}");
}

/// Lead ruling (DIS3): Space Cowboy's `Main DownArrow` has no art; its
/// copy crops `StatusWrapper`'s art, which paints 18 pixels of its 270 px
/// box. Every one of the 18 changes, so the copy is served though fewer
/// than [`VISIBLE_PIXELS`] change.
#[test]
fn spacecowboy_down_arrow_serves_a_copy_changing_every_painted_pixel() {
    let (served_manifest, made) = served("spacecowboy");
    let pkg = pack("spacecowboy");
    let layout = &served_manifest.windows[&SkinWindow::Main];
    let tree = layout.tree.as_ref().unwrap();
    let (win_w, win_h) = target::window(layout);
    let boxes: HashMap<&str, [i32; 4]> = tree
        .boxes(win_w, win_h)
        .into_iter()
        .map(|(n, b)| (n.id.as_str(), b))
        .collect();
    let mut measured = Measured::default();
    let mut from = None;
    walk(&tree.root, None, &boxes, &mut |n, at, under| {
        if n.id == "DownArrow" {
            from = Some(before(n, at, under, &pkg, &mut measured));
        }
    });
    let from = from.unwrap();
    let copy = copy_of(find(&tree.root, "DownArrow").unwrap());
    let normal = &measured.frames[&from[&PieceSlot::Middle].image];
    let dimmed = frame(&copy[&PieceSlot::Middle].image, &made, &pkg);
    assert_eq!(normal.width() * normal.height(), 270);
    let painted: Vec<_> = normal
        .enumerate_pixels()
        .filter(|(_, _, px)| px.0[3] > 0)
        .collect();
    assert_eq!(painted.len(), 18);
    for (x, y, px) in painted {
        assert!(
            visibly_differs(px.0, dimmed.get_pixel(x, y).0),
            "({x}, {y})"
        );
    }
    assert!(visible_difference(normal, &dimmed) < VISIBLE_PIXELS);
}

/// Lead ruling (DIS3): the every-painted-pixel arm is only for art that
/// paints fewer than [`VISIBLE_PIXELS`]; art painting 25 pixels that
/// changes 19 is still refused, and art that paints nothing never passes.
#[test]
fn the_small_art_arm_never_lowers_the_bar_for_large_art() {
    let p = pack("alloy");
    let check = |painted: usize, changed: usize| {
        let mut normal = RgbaImage::new(5, 5);
        for px in normal.pixels_mut().take(painted) {
            px.0 = [100, 100, 100, 255];
        }
        let mut copy = normal.clone();
        for px in copy.pixels_mut().take(changed) {
            px.0[0] = 200;
        }
        let mut measured = Measured::default();
        let set = |name: &str, image: RgbaImage, measured: &mut Measured| {
            measured.frames.insert(name.to_owned(), image);
            NodePieces::from([(
                PieceSlot::Middle,
                NodePiece {
                    image: name.to_owned(),
                    width: 5,
                    height: 5,
                },
            )])
        };
        let from = set("normal", normal, &mut measured);
        let to = set("copy", copy, &mut measured);
        visibly_changed(&from, &to, &p, &mut measured).unwrap()
    };
    assert!(!check(25, 19), "25 painted, 19 changed: under the bar");
    assert!(check(25, 20), "25 painted, 20 changed");
    assert!(check(18, 18), "18 painted, every one changed");
    assert!(!check(18, 17), "18 painted, 17 changed");
    assert!(!check(0, 0), "nothing painted");
}
