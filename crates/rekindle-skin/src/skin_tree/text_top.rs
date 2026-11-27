//! The text-over-art rule (plan E7.7, RV1): a control that shows text
//! paints above every overlapping sibling that is art only.
//!
//! **What Xfire did.** The engine's text views are child windows (the
//! status field is `Class=StatusView`), which draw over the skin window's
//! painted tiles whatever their `Z`, as the chat's message list and
//! composer do ([`super::chat_slots`]). Alloy lays its `StatusWrapper`
//! plate (`Z=4`, art `STATUSBBGMIDDLE`/`STATUSBGRIGHT`) over its
//! `StatusEdit` field (`Z=3`, `Alloy.ini:45–64, 299`), and Simple X the
//! same (`SimpleX.ini:47–70, 343`); both packs shipped working in Xfire,
//! so the plate never covered the field's text. Onyx lays its
//! `StatusEdit` (`Z=22`) above its `StatusWrapper` (18) and is served as
//! the pack lays it.
//!
//! **What the Host serves.** Served `z` is three times the pack's
//! ([`super::validate::stack_z`]), so Alloy's plate (12) covered the
//! field (9): the status text and the paused-route line in it (RV1) were
//! hidden under the plate's middle tile. Each node holding a text control
//! ([`shows_text`]) is raised one above every sibling stacked above it
//! whose subtree is art only (no node of it takes input, [`takes_input`])
//! and whose art meets the text, in the served minimum or a large window.
//! Interactive siblings keep their pack order relative to it (Alloy's
//! `DownArrow`, `Z=5`, stays above the field): one stacked above the
//! holder whose boxes meet the text is raised with it when the lift would
//! pass it.

use std::collections::{BTreeMap, HashMap};

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, SlotKey};

use super::chat_slots::LARGE;
use super::inset::is_text;
use super::target::listens;
use super::validate::{overlap, Apart};

/// Whether the node shows text the user reads or types: the name, status
/// and room fields ([`is_text`]) and the chat composer.
#[must_use]
pub fn shows_text(n: &SkinNode) -> bool {
    is_text(n.role) || n.role == NodeRole::Slot(SlotKey::ChatEdit)
}

/// Whether the node takes input: a press, a field, the status list, a tab.
#[must_use]
pub fn takes_input(n: &SkinNode) -> bool {
    listens(n) || n.interactive()
}

/// Applies the rule to `layout`'s tree and popups.
pub fn serve(layout: &mut WindowLayout) {
    super::titles::each_laid(layout, lay);
}

/// Every node of `n`'s subtree, `n` first.
#[must_use]
pub fn subtree(n: &SkinNode) -> Vec<&SkinNode> {
    let mut out = vec![n];
    for c in &n.children {
        out.extend(subtree(c));
    }
    out
}

/// Each node's box in the served minimum and the large window, by id.
pub type Boxes = HashMap<String, Vec<[i32; 4]>>;

/// The boxes of every node of `tree` in `at` and [`LARGE`].
#[must_use]
pub fn boxes(tree: &SkinTree, at: (i32, i32)) -> Boxes {
    let mut out = Boxes::new();
    for (w, h) in [at, LARGE] {
        for (n, b) in tree.boxes(w, h) {
            out.entry(n.id.clone()).or_default().push(b);
        }
    }
    out
}

/// Whether `a` and `b` overlap in any compared window.
#[must_use]
pub fn meets(boxes: &Boxes, a: &str, b: &str) -> bool {
    match (boxes.get(a), boxes.get(b)) {
        (Some(a), Some(b)) => a.iter().zip(b).any(|(x, y)| overlap(*x, *y)),
        _ => false,
    }
}

fn lay(tree: &mut SkinTree, at: (i32, i32)) {
    let laid = boxes(tree, at);
    let mut raised = BTreeMap::new();
    let apart = Apart::new(&tree.root);
    plan(&tree.root, &Ctx { laid, apart }, &mut raised);
    apply(&mut tree.root, &raised);
}

struct Ctx<'a> {
    laid: Boxes,
    apart: Apart<'a>,
}

/// Whether a node of `texts` meets a node of `other` that `paints` and
/// is shown with it.
fn touches(
    texts: &[&SkinNode],
    other: &[&SkinNode],
    ctx: &Ctx<'_>,
    paints: fn(&SkinNode) -> bool,
) -> bool {
    texts.iter().any(|t| {
        other
            .iter()
            .filter(|o| paints(o))
            .any(|o| !ctx.apart.apart(&t.id, &o.id) && meets(&ctx.laid, &t.id, &o.id))
    })
}

/// Art the text would lie under.
fn has_art(n: &SkinNode) -> bool {
    n.art.is_some()
}

/// A node whose order against the text shows: it draws art or takes a
/// press (a row group with neither draws nothing).
fn drawn_or_pressed(n: &SkinNode) -> bool {
    n.art.is_some() || takes_input(n)
}

/// The new `Z` of each sibling of `nodes` the rule moves, by id, then the
/// same for every level below.
fn plan(nodes: &[SkinNode], ctx: &Ctx<'_>, out: &mut BTreeMap<String, i16>) {
    let subs: Vec<Vec<&SkinNode>> = nodes.iter().map(subtree).collect();
    let mut z: Vec<i16> = nodes.iter().map(|n| n.z).collect();
    for k in 0..nodes.len() {
        let texts: Vec<&SkinNode> = subs[k].iter().copied().filter(|n| shows_text(n)).collect();
        if texts.is_empty() {
            continue;
        }
        let above = |j: usize, z: &[i16]| z[j] > z[k] || (z[j] == z[k] && j > k);
        let top = (0..nodes.len())
            .filter(|&j| j != k && above(j, &z))
            .filter(|&j| !subs[j].iter().any(|n| takes_input(n)))
            .filter(|&j| touches(&texts, &subs[j], ctx, has_art))
            .map(|j| z[j])
            .max();
        let Some(top) = top else { continue };
        let lifted = top.saturating_add(1);
        let keep: Vec<usize> = (0..nodes.len())
            .filter(|&j| j != k && above(j, &z))
            .filter(|&j| subs[j].iter().any(|n| takes_input(n)))
            .filter(|&j| touches(&texts, &subs[j], ctx, drawn_or_pressed))
            .collect();
        z[k] = lifted;
        out.insert(nodes[k].id.clone(), lifted);
        for j in keep {
            if !above(j, &z) {
                let kept = if j > k {
                    lifted
                } else {
                    lifted.saturating_add(1)
                };
                z[j] = kept;
                out.insert(nodes[j].id.clone(), kept);
            }
        }
    }
    for n in nodes {
        plan(&n.children, ctx, out);
    }
}

fn apply(nodes: &mut [SkinNode], raised: &BTreeMap<String, i16>) {
    for n in nodes {
        if let Some(z) = raised.get(&n.id) {
            n.z = *z;
        }
        apply(&mut n.children, raised);
    }
}
