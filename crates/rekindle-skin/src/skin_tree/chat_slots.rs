//! The chat window's serve passes (plan SK17, the real-app round 34
//! findings): the chat title is the pack's own text point, and the
//! conversation's native controls draw above the pack's art.
//!
//! **The title's text point.** A pack that lays its title as a field
//! around a text names the field `UserNameField` and the text inside it
//! `UserNameText` (Alloy's `[UserNameField]` → `[UserNameBG]` →
//! `UserNameText`, Simple X's and XGlow's `[UserNameField]` `Window1.Name=
//! UserNameText X=14`, `ChatSkin.ini:154-161`; xSLIMFIRE's
//! `ChatUserNameField` `X=18 JustX=RIGHT ResizeX=100` around
//! `ChatUserNameText`, `xSLIMFIREchatwindow.xml:4-5`). The role table
//! gives the field the title ([`super::chat_roles`]) and its children are
//! parts, so the shell wrote the name at the field's corner: over Alloy's
//! `UserNameBG` plate (an empty title) and at x 0 of xSLIMFIRE's
//! window-wide field (over its frame edge and logo). The engine draws the
//! name where the text sits; the field's one art-less leaf part is that
//! text, so it takes the title and the field is a group around it
//! ([`super::text_box`] then gives the point-laid text the room its
//! parent declares past it, as Onyx's `Title`).
//!
//! **Native controls above the art.** The engine's message list and
//! composer are child windows (`ChatArea`, `ChatEdit`), which draw over
//! the skin window's painted tiles whatever their `Z`. Alloy's
//! `MainTitleBar` art (300×160, `Z=1`) and `Middle` lie over its
//! `ChatArea` (`Z` 0); Simple X's 70 px title bar art over the top of its
//! own, where the shell mounts the peer's presence row. Each such slot,
//! and each group holding one, is raised above every sibling whose art
//! (its own or a descendant's) meets it.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SlotKey};

/// The large window the slot lift (and [`super::text_top`]) also compares
/// boxes in.
pub(super) const LARGE: (i32, i32) = (1200, 900);

/// Runs both passes on `layout`'s tree (a window without one is left).
pub fn serve(layout: &mut WindowLayout) {
    let at = super::target::window(layout);
    let Some(tree) = layout.tree.as_mut() else {
        return;
    };
    title_leaf(&mut tree.root);
    let laid: Vec<Vec<[i32; 4]>> = [at, LARGE]
        .iter()
        .map(|&(w, h)| tree.boxes(w, h).into_iter().map(|(_, b)| b).collect())
        .collect();
    let mut kinds = Vec::new();
    flat(&tree.root, &mut kinds);
    lift(&mut tree.root, &Laid { laid, kinds }, 0);
}

fn text_leaf(n: &SkinNode) -> bool {
    n.role == NodeRole::Part && n.art.is_none() && n.label.is_none() && n.children.is_empty()
}

fn count_leaves(nodes: &[SkinNode]) -> usize {
    nodes
        .iter()
        .map(|n| usize::from(text_leaf(n)) + count_leaves(&n.children))
        .sum()
}

/// Gives the first text leaf under `nodes` the title; whether it found one.
fn mark_leaf(nodes: &mut [SkinNode]) -> bool {
    nodes.iter_mut().any(|n| {
        if text_leaf(n) {
            n.role = NodeRole::ChatTitle;
            true
        } else {
            mark_leaf(&mut n.children)
        }
    })
}

/// Moves each field-laid chat title onto its one text leaf.
fn title_leaf(nodes: &mut [SkinNode]) {
    for n in nodes {
        if n.role == NodeRole::ChatTitle && count_leaves(&n.children) == 1 {
            n.role = NodeRole::Group;
            mark_leaf(&mut n.children);
        } else {
            title_leaf(&mut n.children);
        }
    }
}

/// A node's box in each compared window, and what it is, depth first.
struct Laid {
    laid: Vec<Vec<[i32; 4]>>,
    /// (a native-control slot, carries art) per node.
    kinds: Vec<(bool, bool)>,
}

fn native(role: NodeRole) -> bool {
    matches!(role, NodeRole::Slot(SlotKey::ChatArea | SlotKey::ChatEdit))
}

fn flat(nodes: &[SkinNode], out: &mut Vec<(bool, bool)>) {
    for n in nodes {
        out.push((native(n.role), n.art.is_some()));
        flat(&n.children, out);
    }
}

fn size(n: &SkinNode) -> usize {
    1 + n.children.iter().map(size).sum::<usize>()
}

impl Laid {
    /// Whether a native slot in the subtree `slots` meets art in `art`.
    fn meets(&self, slots: std::ops::Range<usize>, art: std::ops::Range<usize>) -> bool {
        slots.filter(|&i| self.kinds[i].0).any(|i| {
            art.clone().filter(|&j| self.kinds[j].1).any(|j| {
                self.laid
                    .iter()
                    .any(|boxes| super::validate::overlap(boxes[i], boxes[j]))
            })
        })
    }
}

/// Raises each node of `nodes` (the first at depth-first index `first`)
/// holding a native slot above the siblings whose art meets that slot.
fn lift(nodes: &mut [SkinNode], laid: &Laid, first: usize) {
    let mut ranges = Vec::with_capacity(nodes.len());
    let mut at = first;
    for n in nodes.iter() {
        ranges.push(at..at + size(n));
        at += size(n);
    }
    let raised: Vec<Option<i16>> = nodes
        .iter()
        .zip(&ranges)
        .enumerate()
        .map(|(k, (n, own))| {
            nodes
                .iter()
                .zip(&ranges)
                .enumerate()
                .filter(|(j, (_, other))| *j != k && laid.meets(own.clone(), (*other).clone()))
                .map(|(_, (s, _))| s.z)
                .max()
                .filter(|&top| n.z <= top)
                .map(|top| top.saturating_add(1))
        })
        .collect();
    for ((n, z), own) in nodes.iter_mut().zip(raised).zip(ranges) {
        if let Some(z) = z {
            n.z = z;
        }
        let holds = own.clone().any(|i| laid.kinds[i].0);
        if holds {
            lift(&mut n.children, laid, own.start + 1);
        }
    }
}
