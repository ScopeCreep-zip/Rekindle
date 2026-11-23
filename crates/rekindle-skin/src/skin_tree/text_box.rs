//! The usable box of a node that draws text (plan SK8, the contract with
//! the shell): a pack may lay a text node at a point, 0 px wide on its text
//! axis (Combat's name field `Title` at x 55, size 0; Onyx's
//! `MainTitleBar/Title` at x 20, size 0; SpaceCowboy's `Title` and its menu
//! titles; XGlow's `UserName`), where the engine drew the string from that
//! point; or in a box too small for its word (Onyx's 4 px menu titles).
//! A 0-wide box cannot ellipsize, so a long name ran out of the window.
//!
//! The rule: every art-less node that draws text, a text field
//! ([`is_text`]) or a node carrying a served [`NodeLabel`] with words and no
//! glyph (SK4b menu titles, SK6/SK6b tab and button words, the N4 clone),
//! whose pack span on x is `Fixed`, is served its usable box when the
//! pack's box is narrower:
//!
//! - from its anchor to its parent's end (a `Start` anchor), from its
//!   parent's start to its anchor (`End`), or the widest box about its
//!   centre (`Centre`), never past the parent's box;
//! - stopped at the near edge of an interactive sibling that meets its line
//!   (the node's rows, at least one) and lies wholly past the anchor: the
//!   siblings [`super::inset`] counts (it takes input, is no text field, no
//!   resize grip, not another view's), so the text runs under none. The
//!   sibling's edge is its target box's ([`super::target`], WCAG 2.2 SC
//!   2.5.8, laid on the pack's boxes before any text grows), not its art's:
//!   the 24 px target takes precedence over the text's ellipsis room, so a
//!   name field beside an 18 px button stops 3 px short of the button's
//!   art. The final targets are laid after the text grows (a text-drawn
//!   title that listens itself serves its own target over its grown box);
//!   a button's target then reaches the same edge, since the grown text
//!   stops where it began;
//! - laid at the window's served minimum.
//!
//! A pack box reaching past its parent is served its usable box too, so no
//! text box is wider than its parent's.
//!
//! The served span: a `Stretch` when every sibling past the anchor moves
//! with the parent's far edge (an `End`-anchored one past a `Start` text, a
//! `Start`-anchored one before an `End` text), so the box grows with the
//! window and still meets none; else the same anchor at the larger size.
//! A node with art keeps its box: the art is the box (Alloy's 48 px file
//! title), and its words live in its cover.

use std::collections::HashMap;

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey};

use super::inset::is_text;
use super::titles::{laid_window, place, popup_window};

/// Each small interactive node's target box on the pack's boxes, relative
/// to its art: `[dx, dy, w, h]` by node id.
type Targets = HashMap<String, [i32; 4]>;

fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let (x0, y0) = (a[0].min(b[0]), a[1].min(b[1]));
    let (x1, y1) = (
        (a[0] + a[2]).max(b[0] + b[2]),
        (a[1] + a[3]).max(b[1] + b[3]),
    );
    [x0, y0, x1 - x0, y1 - y0]
}

/// The target boxes [`super::target`] lays for `tree` in `window`, before
/// any text grows.
fn targets(tree: &SkinTree, window: (i32, i32)) -> Targets {
    super::target::targets(tree, window)
        .into_iter()
        .map(|l| {
            let off = [
                l.area[0] - l.art[0],
                l.area[1] - l.art[1],
                l.area[2],
                l.area[3],
            ];
            (l.id, off)
        })
        .collect()
}

/// The box `b` of `n` grown over its target box, if it serves one.
fn reach(n: &SkinNode, b: [i32; 4], targets: &Targets) -> [i32; 4] {
    targets
        .get(&n.id)
        .map_or(b, |t| union(b, [b[0] + t[0], b[1] + t[1], t[2], t[3]]))
}

/// Whether `n` draws text the shell must fit: a text field, or words the
/// Host serves over it. An art node keeps its box.
pub(super) fn draws_text(n: &SkinNode) -> bool {
    n.art.is_none() && (is_text(n.role) || n.label.as_ref().is_some_and(|l| l.glyph.is_none()))
}

/// Whether `by` can stop `n`'s text: it takes input, is no text field or
/// resize grip, and shows with `n`'s view.
fn blocks(n: &SkinNode, by: &SkinNode) -> bool {
    let apart = by.shows_with.is_some() && n.shows_with.is_some() && by.shows_with != n.shows_with;
    !apart && by.interactive() && !is_text(by.role) && by.resize_edge().is_none()
}

/// The box `b` of `n` grown over its descendants' boxes: a text-drawn
/// title's button art overhangs its text (Onyx's `ToolsMenu` part,
/// `Stretch` -4/-4 around its 4 px `ToolsMenuText`), and the text runs
/// under neither.
/// Each descendant counts with its own target box (plan SK12: Onyx's
/// `ToolsMenu` part serves a target 1 px left of its art, and the
/// `XfireMenuText` before it stops there).
fn extent(n: &SkinNode, b: [i32; 4], targets: &Targets) -> [i32; 4] {
    n.children.iter().fold(b, |e, c| {
        let cb = place(c, b);
        union(e, extent(c, reach(c, cb, targets), targets))
    })
}

/// A box no text may run over (plan SK12), laid on the pack's boxes: a
/// blocking node's press area ([`reach`] over [`extent`]), another text's
/// box (its rows, at least one), or the views (the app's content, the
/// buddy list and its search field). The served text stops at the near
/// edge of any of them on its line, wherever it lies in the tree, except
/// its own ancestors and descendants.
struct Obstacle {
    id: String,
    up: Vec<String>,
    shows: Option<ViewKey>,
    /// A blocking node, which [`usable`] already counts among its
    /// siblings.
    blocker: bool,
    b: [i32; 4],
}

fn obstacles(
    nodes: &[SkinNode],
    parent: [i32; 4],
    up: &[String],
    targets: &Targets,
    out: &mut Vec<Obstacle>,
) {
    for n in nodes {
        let b = place(n, parent);
        let blocker = n.interactive() && !is_text(n.role) && n.resize_edge().is_none();
        let area = if blocker {
            Some(reach(n, extent(n, b, targets), targets))
        } else if draws_text(n) {
            Some(reach(n, [b[0], b[1], b[2], b[3].max(1)], targets))
        } else {
            (n.role == NodeRole::Views).then_some(b)
        };
        if let Some(b) = area {
            out.push(Obstacle {
                id: n.id.clone(),
                up: up.to_vec(),
                shows: n.shows_with,
                blocker,
                b,
            });
        }
        let mut own = up.to_vec();
        own.push(n.id.clone());
        obstacles(&n.children, b, &own, targets, out);
    }
}

/// The obstacles that stop `n`'s text (see [`Obstacle`]): not `n`, its
/// ancestors (`up`) or descendants, not another view's, and not a
/// blocking sibling (counted by [`usable`] with its span).
fn stops(n: &SkinNode, up: &[String], all: &[Obstacle]) -> Vec<[i32; 4]> {
    all.iter()
        .filter(|o| {
            let apart = o.shows.is_some() && n.shows_with.is_some() && o.shows != n.shows_with;
            o.id != n.id
                && !o.up.contains(&n.id)
                && !up.contains(&o.id)
                && !apart
                && !(o.blocker && o.up == up)
        })
        .map(|o| o.b)
        .collect()
}

/// Keeps each child of a node that grew from `was` to `now` at its pack
/// box on x: the button art a text-drawn tab or title lays around its
/// words (Classic's `TabNormalButton`, `Stretch` -4/-18 around its 40 px
/// text) keeps the pack's size and place; only the text box grows.
fn pin_children(n: &mut SkinNode, was: [i32; 4], now: [i32; 4]) {
    for c in &mut n.children {
        let (start, size) = c.x.place(was[2]);
        let rel = was[0] + start - now[0];
        if let (Ok(o), Ok(size)) = (i16::try_from(rel), u16::try_from(size)) {
            c.x = NodeSpan::Fixed {
                at: Anchor::Start(o),
                size,
            };
        }
    }
}

/// The usable x span of `nodes[i]` in `parent`, laid at `boxes`, or
/// `None` when the pack's box already fills it.
fn usable(
    i: usize,
    nodes: &[SkinNode],
    boxes: &[[i32; 4]],
    parent: [i32; 4],
    targets: &Targets,
    extra: &[[i32; 4]],
) -> Option<NodeSpan> {
    let NodeSpan::Fixed { at, size } = nodes[i].x else {
        return None;
    };
    let b = boxes[i];
    let (lo, hi) = (parent[0], parent[0] + parent[2]);
    let line = [b[1], b[1] + b[3].max(1)];
    // A whole-tree obstacle has no span beside this node: it never moves
    // with the parent's far edge, so the box stays `Fixed`.
    let near: Vec<(Option<&SkinNode>, [i32; 4])> = nodes
        .iter()
        .zip(boxes)
        .enumerate()
        .filter(|(j, (s, _))| *j != i && blocks(&nodes[i], s))
        .map(|(_, (s, sb))| (Some(s), reach(s, extent(s, *sb, targets), targets)))
        .chain(extra.iter().map(|e| (None, *e)))
        .filter(|(_, sb)| sb[1] < line[1] && line[0] < sb[1] + sb[3])
        .collect();
    let fixed = |len: i32| {
        let size = u16::try_from(len).ok()?;
        (size > 0).then_some(NodeSpan::Fixed { at, size })
    };
    // Served when it grows the box, or when the pack's box reaches past
    // its parent (Classic's `HelpMenuText`, x 269-368 in a parent ending at
    // x 343): the usable box is never wider than the parent's.
    let out = b[0] < lo || b[0] + b[2] > hi;
    let serve = |len: i32| len > i32::from(size) || (out && len < i32::from(size));
    match at {
        Anchor::Start(o) => {
            let past: Vec<_> = near.iter().filter(|(_, sb)| sb[0] >= b[0]).collect();
            let end = past.iter().map(|(_, sb)| sb[0]).fold(hi, i32::min);
            let moves = past.iter().all(|(s, _)| {
                s.is_some_and(|s| {
                    matches!(
                        s.x,
                        NodeSpan::Fixed {
                            at: Anchor::End(_),
                            ..
                        }
                    )
                })
            });
            if !serve(end - b[0]) {
                return None;
            }
            if moves {
                let end = i16::try_from(hi - end).ok()?;
                Some(NodeSpan::Stretch { start: o, end })
            } else {
                fixed(end - b[0])
            }
        }
        Anchor::End(o) => {
            let right = b[0] + b[2];
            let before: Vec<_> = near
                .iter()
                .filter(|(_, sb)| sb[0] + sb[2] <= right)
                .collect();
            let start = before
                .iter()
                .map(|(_, sb)| sb[0] + sb[2])
                .fold(lo, i32::max);
            let moves = before.iter().all(|(s, _)| {
                s.is_some_and(|s| {
                    matches!(
                        s.x,
                        NodeSpan::Fixed {
                            at: Anchor::Start(_),
                            ..
                        }
                    )
                })
            });
            if !serve(right - start) {
                return None;
            }
            if moves {
                let start = i16::try_from(start - lo).ok()?;
                Some(NodeSpan::Stretch { start, end: o })
            } else {
                fixed(right - start)
            }
        }
        Anchor::Centre { .. } => {
            let mid = 2 * b[0] + b[2];
            let mut room = (mid - 2 * lo).min(2 * hi - mid);
            for (_, sb) in &near {
                if 2 * sb[0] >= mid {
                    room = room.min(2 * sb[0] - mid);
                } else if 2 * (sb[0] + sb[2]) <= mid {
                    room = room.min(mid - 2 * (sb[0] + sb[2]));
                }
            }
            // Xfire floors the centred start: shrink until the box fits.
            let fits = |len: i32| {
                let (s, w) = NodeSpan::Fixed {
                    at,
                    size: u16::try_from(len).unwrap_or(0),
                }
                .place(parent[2]);
                2 * (lo + s) >= mid - room && 2 * (lo + s + w) <= mid + room
            };
            let len = (0..=room).rev().find(|l| fits(*l))?;
            if serve(len) {
                fixed(len)
            } else {
                None
            }
        }
    }
}

fn walk(
    nodes: &mut [SkinNode],
    parent: [i32; 4],
    up: &[String],
    targets: &Targets,
    all: &[Obstacle],
) {
    let boxes: Vec<[i32; 4]> = nodes.iter().map(|n| place(n, parent)).collect();
    let spans: Vec<Option<NodeSpan>> = (0..nodes.len())
        .map(|i| {
            draws_text(&nodes[i])
                .then(|| {
                    let extra = stops(&nodes[i], up, all);
                    usable(i, nodes, &boxes, parent, targets, &extra)
                })
                .flatten()
        })
        .collect();
    for ((n, span), was) in nodes.iter_mut().zip(spans).zip(boxes) {
        if let Some(x) = span {
            n.x = x;
            pin_children(n, was, place(n, parent));
        }
        let b = place(n, parent);
        let mut own = up.to_vec();
        own.push(n.id.clone());
        walk(&mut n.children, b, &own, targets, all);
    }
}

/// Lays `tree`'s text boxes in `window` (see the module doc).
fn lay_tree(tree: &mut SkinTree, window: (i32, i32)) {
    let targets = targets(tree, window);
    let at = laid_window(tree.outsets, window);
    let mut all = Vec::new();
    obstacles(&tree.root, at, &[], &targets, &mut all);
    walk(&mut tree.root, at, &[], &targets, &all);
}

/// Serves every text-drawing node of `layout`'s tree and popups its usable
/// box (see the module doc), laid at the window's minimum size.
pub fn lay(layout: &mut WindowLayout) {
    let at = super::target::window(layout);
    if let Some(tree) = layout.tree.as_mut() {
        lay_tree(tree, at);
    }
    let min_height = layout.min_height;
    for popup in layout.popups.values_mut() {
        let at = popup_window(popup, min_height);
        lay_tree(&mut popup.tree, at);
    }
}
