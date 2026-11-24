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
use super::titles::{each_laid, laid_window, place};
use super::validate::{overlap, Apart};

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
pub(super) struct Obstacle {
    pub(super) id: String,
    pub(super) up: Vec<String>,
    shows: Option<ViewKey>,
    /// A blocking node, which [`usable`] already counts among its
    /// siblings.
    blocker: bool,
    pub(super) b: [i32; 4],
    /// The node's own box ([`extent`] for a blocker), before [`reach`]
    /// grows it over its target (see [`on_line`]).
    art: [i32; 4],
}

impl Obstacle {
    /// This stop's box as it stops a content text's line ([`on_line`]).
    pub(super) fn on_content_line(&self) -> [i32; 4] {
        on_line(self.b, self.art, true)
    }
}

/// The box `b` of a stop whose own box is `art`, as it stops a text that
/// `keeps_line` or not. A content text's line is the shell's line box at
/// the pack's point (plan SK15b-2, [`super::content`]): a press area's
/// target padding above or below its art does not cut that line (XGlow's
/// `Status` art starts 3 px under the name's line, `Y="UserName.BOTTOM+2"`
/// then `+1`, `Xglow.ini:195-217`; its 24 px target reaches 2 px into it),
/// while its padding on the line's own axis still stops the words (plan
/// SK12: Onyx's `ToolsMenu` target 1 px left of its art).
fn on_line(b: [i32; 4], art: [i32; 4], keeps_line: bool) -> [i32; 4] {
    if keeps_line {
        [b[0], art[1], b[2], art[3]]
    } else {
        b
    }
}

/// Every obstacle of `tree` laid in `window` (see [`Obstacle`]), on the
/// boxes as laid: what a content text's row may not reach (plan SK15b-2,
/// [`super::content`]).
pub(super) fn obstacles_of(tree: &SkinTree, window: (i32, i32)) -> Vec<Obstacle> {
    let targets = targets(tree, window);
    let mut all = Vec::new();
    let at = laid_window(tree.outsets, window);
    obstacles(&tree.root, at, &[], (&targets, None), &mut all);
    all
}

fn obstacles(
    nodes: &[SkinNode],
    parent: [i32; 4],
    up: &[String],
    (targets, view): (&Targets, Option<ViewKey>),
    out: &mut Vec<Obstacle>,
) {
    for n in nodes {
        let b = place(n, parent);
        // A node shows with its own view or its nearest ancestor's (Simple-X's
        // `BigBrowseFileText` under the Files row's `SearchFiles`).
        let shows = n.shows_with.or(view);
        let blocker = n.interactive() && !is_text(n.role) && n.resize_edge().is_none();
        let own = if blocker {
            Some(extent(n, b, targets))
        } else if draws_text(n) {
            Some([b[0], b[1], b[2], b[3].max(1)])
        } else {
            (n.role == NodeRole::Views).then_some(b)
        };
        if let Some(art) = own {
            let reached = if n.role == NodeRole::Views {
                art
            } else {
                reach(n, art, targets)
            };
            out.push(Obstacle {
                id: n.id.clone(),
                up: up.to_vec(),
                shows,
                blocker,
                b: reached,
                art,
            });
        }
        let mut own = up.to_vec();
        own.push(n.id.clone());
        obstacles(&n.children, b, &own, (targets, shows), out);
    }
}

/// The obstacles that stop `n`'s text (see [`Obstacle`]): not `n`, its
/// ancestors (`up`) or descendants, not another view's, and not a
/// blocking sibling (counted by [`usable`] with its span).
/// With `siblings`, the blocking siblings count too (the plan SK13 cut).
fn stops(n: &SkinNode, up: &[String], all: &[Obstacle], siblings: bool) -> Vec<[i32; 4]> {
    let mine = all
        .iter()
        .find(|o| o.id == n.id && o.up == up)
        .map_or(n.shows_with, |o| o.shows);
    all.iter()
        .filter(|o| {
            let apart = o.shows.is_some() && mine.is_some() && o.shows != mine;
            let kin = o.id == n.id || o.up.contains(&n.id) || up.contains(&o.id);
            let sibling_blocker = !siblings && o.blocker && o.up == up;
            !(kin || apart || sibling_blocker)
        })
        .map(|o| on_line(o.b, o.art, n.fit.is_some()))
        .collect()
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

/// Lays `tree`'s text boxes in `window` (see the module doc).
fn lay_tree(tree: &mut SkinTree, window: (i32, i32)) {
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
