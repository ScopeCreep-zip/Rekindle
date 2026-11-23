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
//!   resize grip, not another view's), so the text runs under none;
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

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode};

use super::inset::is_text;
use super::titles::{each_laid, place};

/// Whether `n` draws text the shell must fit: a text field, or words the
/// Host serves over it. An art node keeps its box.
fn draws_text(n: &SkinNode) -> bool {
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
fn extent(n: &SkinNode, b: [i32; 4]) -> [i32; 4] {
    n.children.iter().fold(b, |e, c| {
        let k = extent(c, place(c, b));
        let (x0, y0) = (e[0].min(k[0]), e[1].min(k[1]));
        let (x1, y1) = (
            (e[0] + e[2]).max(k[0] + k[2]),
            (e[1] + e[3]).max(k[1] + k[3]),
        );
        [x0, y0, x1 - x0, y1 - y0]
    })
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
fn usable(i: usize, nodes: &[SkinNode], boxes: &[[i32; 4]], parent: [i32; 4]) -> Option<NodeSpan> {
    let NodeSpan::Fixed { at, size } = nodes[i].x else {
        return None;
    };
    let b = boxes[i];
    let (lo, hi) = (parent[0], parent[0] + parent[2]);
    let line = [b[1], b[1] + b[3].max(1)];
    let near: Vec<(&SkinNode, [i32; 4])> = nodes
        .iter()
        .zip(boxes)
        .enumerate()
        .filter(|(j, (s, _))| *j != i && blocks(&nodes[i], s))
        .map(|(_, (s, sb))| (s, extent(s, *sb)))
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
                matches!(
                    s.x,
                    NodeSpan::Fixed {
                        at: Anchor::End(_),
                        ..
                    }
                )
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
                matches!(
                    s.x,
                    NodeSpan::Fixed {
                        at: Anchor::Start(_),
                        ..
                    }
                )
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

fn walk(nodes: &mut [SkinNode], parent: [i32; 4]) {
    let boxes: Vec<[i32; 4]> = nodes.iter().map(|n| place(n, parent)).collect();
    let spans: Vec<Option<NodeSpan>> = (0..nodes.len())
        .map(|i| {
            draws_text(&nodes[i])
                .then(|| usable(i, nodes, &boxes, parent))
                .flatten()
        })
        .collect();
    for ((n, span), was) in nodes.iter_mut().zip(spans).zip(boxes) {
        if let Some(x) = span {
            n.x = x;
            pin_children(n, was, place(n, parent));
        }
        let b = place(n, parent);
        walk(&mut n.children, b);
    }
}

/// Serves every text-drawing node of `layout`'s tree and popups its usable
/// box (see the module doc), laid at the window's minimum size.
pub fn lay(layout: &mut WindowLayout) {
    each_laid(layout, walk);
}
