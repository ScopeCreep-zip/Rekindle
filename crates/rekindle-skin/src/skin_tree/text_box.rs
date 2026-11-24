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
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, SkinTree, SlotKey};

use super::inset::is_text;
use super::titles::{each_laid, laid_window, place};
use super::validate::{overlap, Apart};

mod obstacle;

use obstacle::{obstacles, on_line, stops};
pub(super) use obstacle::{obstacles_of, Obstacle};

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

/// Keeps each child of a node that grew from `was` to `now` at its pack
/// box on x (`axis` 0) or y (`axis` 1): the button art a text-drawn tab or
/// title lays around its words (Classic's `TabNormalButton`, `Stretch`
/// -4/-18 around its 40 px text) keeps the pack's size and place; only the
/// text box changes.
fn pin_children(n: &mut SkinNode, axis: usize, was: [i32; 4], now: [i32; 4]) {
    for c in &mut n.children {
        let span = if axis == 0 { &mut c.x } else { &mut c.y };
        let (start, size) = span.place(was[axis + 2]);
        let rel = was[axis] + start - now[axis];
        if let (Ok(o), Ok(size)) = (i16::try_from(rel), u16::try_from(size)) {
            *span = NodeSpan::Fixed {
                at: Anchor::Start(o),
                size,
            };
        }
    }
}

/// `text` cut back off `stop` on one axis (0 x, 1 y): the larger of the
/// parts before and after `stop`, at least 0 long.
fn cut_on(text: [i32; 4], stop: [i32; 4], axis: usize) -> [i32; 4] {
    let (start, end) = (text[axis], text[axis] + text[axis + 2]);
    let before = (start, stop[axis].min(end));
    let after = ((stop[axis] + stop[axis + 2]).max(start), end);
    let (kept_start, kept_end) = if before.1 - before.0 >= after.1 - after.0 {
        before
    } else {
        after
    };
    let mut kept = text;
    kept[axis] = kept_start;
    kept[axis + 2] = (kept_end - kept_start).max(0);
    kept
}

/// `t` cut off each of `stops` it meets on the axis that keeps it largest
/// (x on a tie: a single-line label keeps its row).
pub(super) fn cut(mut t: [i32; 4], stops: &[[i32; 4]]) -> [i32; 4] {
    for o in stops {
        if overlap(t, *o) {
            let (x, y) = (cut_on(t, *o, 0), cut_on(t, *o, 1));
            t = if y[2] * y[3] > x[2] * x[3] { y } else { x };
        }
    }
    t
}

/// The span of a node that moved from `was` to `now` (start, length) in a
/// parent `len` px long, kept on the same anchor where it has one.
fn respan(span: &NodeSpan, len: i32, was: (i32, i32), now: (i32, i32)) -> Option<NodeSpan> {
    let span = span.clone();
    let (ds, de) = (now.0 - was.0, was.0 + was.1 - now.0 - now.1);
    let size = u16::try_from(now.1).ok()?;
    let add = |o: i16, d: i32| i16::try_from(i32::from(o) + d).ok();
    Some(match span {
        NodeSpan::Stretch { start, end } => NodeSpan::Stretch {
            start: add(start, ds)?,
            end: add(end, de)?,
        },
        NodeSpan::Fixed {
            at: Anchor::End(o), ..
        } => NodeSpan::Fixed {
            at: Anchor::End(add(o, de)?),
            size,
        },
        NodeSpan::Fixed { .. } => NodeSpan::Fixed {
            at: Anchor::Start(i16::try_from(span.place(len).0 + ds).ok()?),
            size,
        },
        // A share moved whole keeps its share at the moved start; a share
        // resized to a px length is no longer one: it keeps its place.
        NodeSpan::Share { permille, .. } if was.1 == now.1 => NodeSpan::Share {
            at: Anchor::Start(i16::try_from(span.place(len).0 + ds).ok()?),
            permille,
        },
        NodeSpan::Share { .. } => return None,
    })
}

/// Plan SK13: cuts the text node at index path `path` off every press area
/// and text box it still meets (see [`stops`], the blocking siblings
/// included), on both axes, even where the pack's arts overlap.
fn cut_one(
    nodes: &mut [SkinNode],
    path: &[usize],
    parent: [i32; 4],
    up: &[String],
    all: &[Obstacle],
) {
    let Some((&i, rest)) = path.split_first() else {
        return;
    };
    let b = place(&nodes[i], parent);
    if !rest.is_empty() {
        let mut own = up.to_vec();
        own.push(nodes[i].id.clone());
        return cut_one(&mut nodes[i].children, rest, b, &own, all);
    }
    let row = [b[0], b[1], b[2], b[3].max(1)];
    let t = cut(row, &stops(&nodes[i], up, all, true));
    let n = &mut nodes[i];
    for axis in [0, 1] {
        if t[axis] == row[axis] && t[axis + 2] == row[axis + 2] {
            continue;
        }
        let was = place(n, parent);
        let span = if axis == 0 { n.x.clone() } else { n.y.clone() };
        let len = parent[axis + 2];
        let moved = (t[axis] - parent[axis], t[axis + 2]);
        let from = (was[axis] - parent[axis], was[axis + 2]);
        if let Some(s) = respan(&span, len, from, moved) {
            if axis == 0 {
                n.x = s;
            } else {
                n.y = s;
            }
            pin_children(n, axis, was, place(n, parent));
        }
    }
}

/// The index paths of `nodes`' text-drawing nodes, in tree order.
fn text_paths(nodes: &[SkinNode], at: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, n) in nodes.iter().enumerate() {
        at.push(i);
        // A text field keeps its box and stops through its inset
        // ([`super::inset`]), so its press area stays whole.
        if draws_text(n) && !is_text(n.role) {
            out.push(at.clone());
        }
        text_paths(&n.children, at, out);
        at.pop();
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
    let NodeSpan::Fixed { at, size } = nodes[i].x.clone() else {
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
        .map(|(_, (s, sb))| {
            let art = extent(s, *sb, targets);
            let b = reach(s, art, targets);
            (Some(s), on_line(b, art, nodes[i].fit.is_some()))
        })
        .chain(extra.iter().map(|e| (None, *e)))
        .filter(|(_, sb)| sb[1] < line[1] && line[0] < sb[1] + sb[3])
        .collect();
    let fixed = |len: i32| {
        let size = u16::try_from(len).ok()?;
        (size > 0).then_some(NodeSpan::Fixed {
            at: at.clone(),
            size,
        })
    };
    // Served when it grows the box, or when the pack's box reaches past
    // its parent (Classic's `HelpMenuText`, x 269-368 in a parent ending at
    // x 343): the usable box is never wider than the parent's.
    let out = b[0] < lo || b[0] + b[2] > hi;
    let serve = |len: i32| len > i32::from(size) || (out && len < i32::from(size));
    match at.clone() {
        Anchor::Start(o) | Anchor::After { gap: o, .. } => {
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
                    at: at.clone(),
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
                    let extra = stops(&nodes[i], up, all, false);
                    usable(i, nodes, &boxes, parent, targets, &extra)
                })
                .flatten()
        })
        .collect();
    for ((n, span), was) in nodes.iter_mut().zip(spans).zip(boxes) {
        if let Some(x) = span {
            n.x = x;
            pin_children(n, 0, was, place(n, parent));
        }
        let b = place(n, parent);
        let mut own = up.to_vec();
        own.push(n.id.clone());
        walk(&mut n.children, b, &own, targets, all);
    }
}

/// The height of the shell's line box for the text `role` draws, px (plan
/// SK15a): the name at 12 px bold (`skin-tree.css` `.skin-name-text`,
/// `font-size: 12px; font-weight: 700`) takes 15 px; every other text, the
/// status line and the Host's labels at 11 px (`.skin-name-edit`,
/// `.skin-status-edit` and `.skin-node-label`, `font-size: 11px`), 14 px.
pub(super) const fn line_box(role: NodeRole) -> u16 {
    match role {
        NodeRole::NameField => 15,
        _ => 14,
    }
}

/// The y span serving `n` its line box when it draws text in a box shorter
/// than that (Onyx's `MainTitleBar/Title` is 0 px tall): the same anchor at
/// the line's height, so it grows away from its anchor (down from a top,
/// up from a bottom, about a centre), as the engine drew the string from
/// that point.
fn line_span(n: &SkinNode) -> Option<NodeSpan> {
    let NodeSpan::Fixed { at, size } = &n.y else {
        return None;
    };
    let line = line_box(n.role);
    (draws_line(n) && *size < line).then(|| NodeSpan::Fixed {
        at: at.clone(),
        size: line,
    })
}

/// Whether `n` draws one line of text: a text node ([`draws_text`]) or the
/// typing slot, where the shell writes one line ("… is typing"; xSLIMFIRE's
/// `ChatTypingMessage` has no art and is 0 px tall in its pack), served
/// the 14 px line box of [`line_box`].
fn draws_line(n: &SkinNode) -> bool {
    draws_text(n) || (n.art.is_none() && n.role == NodeRole::Slot(SlotKey::TypingNotice))
}

/// Serves every text node shorter than its line box that line box
/// ([`line_span`]), before any obstacle is measured, so the text pass
/// (SK8), the targets and the overlap invariant (SK12) see the line the
/// shell draws, not a 0 px row.
fn lines(nodes: &mut [SkinNode], parent: [i32; 4]) {
    for n in nodes {
        let was = place(n, parent);
        if let Some(y) = line_span(n) {
            n.y = y;
            pin_children(n, 1, was, place(n, parent));
        }
        let b = place(n, parent);
        lines(&mut n.children, b);
    }
}

/// Lays `tree`'s text boxes in `window` (see the module doc).
fn lay_tree(tree: &mut SkinTree, window: (i32, i32)) {
    lines(&mut tree.root, laid_window(tree.outsets, window));
    let targets = targets(tree, window);
    let at = laid_window(tree.outsets, window);
    let mut all = Vec::new();
    obstacles(&tree.root, at, &[], (&targets, None), &mut all);
    walk(&mut tree.root, at, &[], &targets, &all);
    // Plan SK13: each text, in tree order, then stops at what it still
    // meets, laid on the boxes the texts before it were cut to.
    let mut paths = Vec::new();
    text_paths(&tree.root, &mut Vec::new(), &mut paths);
    for path in paths {
        let mut all = Vec::new();
        obstacles(&tree.root, at, &[], (&targets, None), &mut all);
        // A node on another page of a button bar or control group is
        // never shown with the text ([`Apart`], the overlap invariant's
        // own rule 4): StealthX's `HostChatButton`, `JoinChatButton` and
        // `LeaveChatButton` share one place in the `InfoJoinChatTab` bar,
        // one shown at a time, so each label keeps its own button's room.
        let apart = Apart::new(&tree.root);
        let id = node_at(&tree.root, &path).id.as_str();
        all.retain(|o| !apart.apart(id, &o.id));
        cut_one(&mut tree.root, &path, at, &[], &all);
    }
}

/// The node at index path `path` (non-empty) under `nodes`.
fn node_at<'a>(nodes: &'a [SkinNode], path: &[usize]) -> &'a SkinNode {
    let (&i, rest) = path.split_first().expect("a text path is never empty");
    if rest.is_empty() {
        &nodes[i]
    } else {
        node_at(&nodes[i].children, rest)
    }
}

/// Serves every text-drawing node of `layout`'s tree and popups its usable
/// box (see the module doc), laid at the window's minimum size.
pub fn lay(layout: &mut WindowLayout) {
    each_laid(layout, lay_tree);
}
