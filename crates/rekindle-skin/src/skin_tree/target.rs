//! The target box of a small interactive node (plan SK7, WCAG 2.2 SC 2.5.8
//! "Target Size (Minimum)", 24 x 24 CSS px): axe's `target-size` measures
//! the element's own box, so the 24 px area is the node's box, never a
//! pseudo-element, and the art keeps its own size inside it.
//!
//! - A candidate is a node that listens itself ([`listens`]: it carries a
//!   press trigger of its own, or takes typed input as a text field or the
//!   status list, the nodes the shell marks `data-skin-on`): a node
//!   interactive only through a part (a text-drawn tab over the
//!   `TabNormalButton` part that carries its press) lets the press fall
//!   through to that part, so a target box on it would catch nothing and
//!   its part serves its own. It is not a resize grip (served by
//!   [`grip_targets`], after every other candidate) and is not
//!   text-sized (a menu, tab or edit field sized by its text at render,
//!   [`text_sized`]), with a box, laid at the window's served minimum.
//! - Its wanted box is its art box grown to 24 px on each short axis about
//!   its centre and shifted inward to lie in the window ([`hit_area`]).
//! - A neighbour is another candidate shown with it (not [`Apart`]) whose
//!   art box does not meet its art box (two the pack overlaps keep their
//!   pack's stacking, rule 4). Where the wanted box meets a neighbour's
//!   wanted box, it is cut along the axis the two art boxes are apart on
//!   (the larger gap when apart on both, x on a tie; both nodes choose the
//!   same axis): it takes half the gap (floored), or the whole gap less
//!   what the neighbour wants of it when that is under half ([`clip`]). The
//!   two target boxes then never overlap, whatever order they are laid in.
//! - A box cut short of 24 px on one side then takes its 24 px from the
//!   free room on its other side, or from a neighbour's share of the gap
//!   the neighbour can spare ([`super::settle`], plan SK15): the target
//!   need not be centred on its art, and the art never moves.
//! - Its rule: `Size` when the box is at least 24 x 24; else `Spacing` when
//!   the 24 px circle centred on its box meets no neighbour's target box and
//!   no undersized neighbour's circle (the criterion's spacing exception;
//!   a box cut at a neighbour touches it, so its art box is tried as the
//!   undersized target instead);
//!   else `Short`, neither holds: a pack-layout finding the Host still serves
//!   (the box it could grow to) and reports.
//! - A listening node the pack lays over its art (not a neighbour): the one
//!   drawn above in the served stack ([`Stack::above`]) keeps its whole
//!   press area and the one below loses the part that area covers
//!   ([`under`], plan SK12), so the two press areas never overlap and a
//!   press lands on the art the user sees on top; the arts draw in the
//!   pack's order.
//! - A node already at least 24 x 24 is unchanged and serves no target,
//!   unless [`under`] cuts it.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::NodeTrigger;
use rekindle_types::dto::skin_tree::{NodeRole, NodeTarget, SkinNode, SkinTree, TargetRule};

use super::{keep, settle};

mod grip;

use super::validate::{hit_area, overlap, text_sized, Apart, Stack, HIT};
#[cfg(test)]
pub(super) use grip::press_box;
pub use grip::{band, grip_holes, grip_targets, grips, Grip};

/// One candidate laid in the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Laid {
    pub id: String,
    /// Its art box `[x, y, w, h]` in the window.
    pub art: [i32; 4],
    /// Its target box in the window.
    pub area: [i32; 4],
    /// How it meets SC 2.5.8.
    pub rule: TargetRule,
}

/// Whether `n` takes a press on its own box: a press trigger of its own,
/// or the typed input of a text field or the status list (the shell's
/// `data-skin-on`, `SkinTreeWindow.tsx`), never only through a part.
#[must_use]
pub fn listens(n: &SkinNode) -> bool {
    n.triggers.iter().any(NodeTrigger::presses)
        || matches!(
            n.role,
            NodeRole::NameField | NodeRole::StatusText | NodeRole::StatusList
        )
}

/// The target box in the window of a node at `art` serving `t`.
#[must_use]
pub fn outer(art: [i32; 4], t: NodeTarget) -> [i32; 4] {
    [art[0] + t.area[0], art[1] + t.area[1], t.area[2], t.area[3]]
}

/// Where `b` lies from `a` along `axis`: after it (`true`) or before it,
/// and the gap between them; `None` when they meet along it.
pub(super) fn side(a: [i32; 4], b: [i32; 4], axis: usize) -> Option<(bool, i32)> {
    let (a0, a1) = (a[axis], a[axis] + a[axis + 2]);
    let (b0, b1) = (b[axis], b[axis] + b[axis + 2]);
    if b0 >= a1 {
        Some((true, b0 - a1))
    } else if b1 <= a0 {
        Some((false, a0 - b1))
    } else {
        None
    }
}

/// How far `want` reaches past `a` along `axis`, after it or before it.
fn reach(a: [i32; 4], want: [i32; 4], axis: usize, after: bool) -> i32 {
    if after {
        want[axis] + want[axis + 2] - (a[axis] + a[axis + 2])
    } else {
        a[axis] - want[axis]
    }
}

/// Cuts `g` (the target box of the node at `a`) where it meets the wanted
/// box `want_b` of the neighbour at `b` (see the module doc).
fn clip(g: &mut [i32; 4], a: [i32; 4], b: [i32; 4], want_b: [i32; 4]) {
    if !overlap(*g, want_b) {
        return;
    }
    let (axis, after, gap) = match (side(a, b, 0), side(a, b, 1)) {
        (Some((ax, gx)), Some((ay, gy))) if gy > gx => (1, ay, gy),
        (Some((ax, gx)), _) => (0, ax, gx),
        (None, Some((ay, gy))) => (1, ay, gy),
        (None, None) => return,
    };
    let theirs = reach(b, want_b, axis, !after);
    let mine = if theirs < gap / 2 {
        gap - theirs
    } else {
        gap / 2
    };
    if after {
        let end = a[axis] + a[axis + 2] + mine;
        g[axis + 2] = g[axis + 2].min(end - g[axis]);
    } else {
        let start = a[axis] - mine;
        if g[axis] < start {
            g[axis + 2] -= start - g[axis];
            g[axis] = start;
        }
    }
}

/// Cuts `g` (the target box of the node at `a`) clear of `top`, the press
/// area of a listening node drawn above it whose art overlaps `a`'s (plan
/// SK12, lead decision: the press goes to what the user sees on top, so
/// press areas never overlap even where the arts do; Classic's slanted tabs
/// overlap by 7 px). `g` keeps the largest of its strips beside `top`
/// (left of it, right, above, below: the cut along the axis that keeps it
/// largest) that still meets `a`, the first in that order on a tie; the
/// upper node keeps its whole area. A `top` that leaves no such strip (two
/// arts in one box, Symbiosis's `FriendChatButton` and `OpenFiles`) leaves
/// `g` as the pack has it. The arts still draw in the pack's order (rule 4).
fn under(g: &mut [i32; 4], a: [i32; 4], top: [i32; 4]) {
    if !overlap(*g, top) {
        return;
    }
    let (g1, g3, t1, t3) = (g[0] + g[2], g[1] + g[3], top[0] + top[2], top[1] + top[3]);
    let strips = [
        [g[0], g[1], top[0] - g[0], g[3]],
        [t1, g[1], g1 - t1, g[3]],
        [g[0], g[1], g[2], top[1] - g[1]],
        [g[0], t3, g[2], g3 - t3],
    ];
    if let Some(s) = strips
        .into_iter()
        .filter(|s| s[2] > 0 && s[3] > 0 && overlap(*s, a))
        .rev()
        .max_by_key(|s| i64::from(s[2]) * i64::from(s[3]))
    {
        *g = s;
    }
}

/// The squared distance, in half pixels, from the centre of `c` to `r`.
fn far(c: [i32; 4], r: [i32; 4]) -> i64 {
    let gap = |at: i32, len: i32, lo: i32, size: i32| {
        let mid = i64::from(2 * at + len);
        let (lo, hi) = (i64::from(2 * lo), i64::from(2 * (lo + size)));
        (lo - mid).max(mid - hi).max(0)
    };
    let dx = gap(c[0], c[2], r[0], r[2]);
    let dy = gap(c[1], c[3], r[1], r[3]);
    dx * dx + dy * dy
}

/// The squared distance, in half pixels, between the centres of `a` and `b`.
fn centres(a: [i32; 4], b: [i32; 4]) -> i64 {
    let dx = i64::from(2 * a[0] + a[2] - 2 * b[0] - b[2]);
    let dy = i64::from(2 * a[1] + a[3] - 2 * b[1] - b[3]);
    dx * dx + dy * dy
}

const fn small(r: [i32; 4]) -> bool {
    r[2] < HIT || r[3] < HIT
}

/// Every candidate of `tree` laid in a `window` (width, height) with its
/// target box and rule (see the module doc).
#[must_use]
pub fn targets(tree: &SkinTree, window: (i32, i32)) -> Vec<Laid> {
    let apart = Apart::new(&tree.root);
    let live: Vec<(&SkinNode, [i32; 4])> = tree
        .boxes(window.0, window.1)
        .into_iter()
        .filter(|(n, b)| {
            listens(n) && n.resize_edge().is_none() && !text_sized(n) && b[2] > 0 && b[3] > 0
        })
        .collect();
    let want: Vec<[i32; 4]> = live.iter().map(|(_, b)| hit_area(*b, window)).collect();
    // Plan SK12: no target grows over the views or a served text box.
    let kept = keep::boxes(tree, window);
    let keeps = |i: usize, id: &str| {
        !apart.apart(&live[i].0.id, id) && !keep::kin(&tree.root, live[i].0, id)
    };
    let near = |i: usize, j: usize| {
        i != j && !apart.apart(&live[i].0.id, &live[j].0.id) && !overlap(live[i].1, live[j].1)
    };
    let laid_over = |i: usize, j: usize| {
        i != j && !apart.apart(&live[i].0.id, &live[j].0.id) && overlap(live[i].1, live[j].1)
    };
    // The spacing exception's circle: 24 px across, 12 px (24 half
    // pixels) from its centre.
    let r2 = i64::from(HIT) * i64::from(HIT);
    let spaced = |i: usize, own: [i32; 4], area: &[[i32; 4]]| {
        (0..live.len())
            .filter(|&j| near(i, j) || laid_over(i, j))
            .all(|j| far(own, area[j]) >= r2 && !(small(area[j]) && centres(own, area[j]) < 4 * r2))
    };
    let mut area: Vec<[i32; 4]> = (0..live.len())
        .map(|i| {
            let mut g = want[i];
            for j in (0..live.len()).filter(|&j| near(i, j)) {
                clip(&mut g, live[i].1, live[j].1, want[j]);
            }
            for (_, o, views) in kept.iter().filter(|(id, ..)| keeps(i, id)) {
                keep::keep_out(&mut g, live[i].1, *o, *views);
            }
            g
        })
        .collect();
    // Plan SK15: a box cut short on one side takes its 24 px from the free
    // room on its other side, or from a neighbour that can spare it.
    let arts: Vec<[i32; 4]> = live.iter().map(|(_, b)| *b).collect();
    settle::settle(&mut area, &arts, &near, &kept, &keeps, window, &spaced);
    // Plan SK12: top of the served stack first, so each node is cut clear
    // of the final press areas of the nodes drawn over its art.
    let stack = Stack::new(&tree.root);
    let mut order: Vec<usize> = (0..live.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (&live[i].0.id, &live[j].0.id);
        if stack.above(a, b) {
            std::cmp::Ordering::Less
        } else if stack.above(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    for (k, &i) in order.iter().enumerate() {
        for &j in order[..k].iter().filter(|&&j| laid_over(i, j)) {
            let top = area[j];
            under(&mut area[i], live[i].1, top);
        }
    }
    // A box cut short of 24 px meets its neighbour's, so its circle never
    // clears it: the node then keeps its art box as its (undersized)
    // target when that clears every neighbour (the spacing exception on
    // the art, as the criterion measures an undersized target). A node
    // whose art meets another listening art keeps its box: its art box
    // would meet the other's press area ([`under`]).
    let area: Vec<[i32; 4]> = (0..live.len())
        .map(|i| {
            let art = live[i].1;
            let whole = !(0..live.len()).any(|j| laid_over(i, j));
            if whole && small(area[i]) && !spaced(i, area[i], &area) && spaced(i, art, &area) {
                art
            } else {
                area[i]
            }
        })
        .collect();
    (0..live.len())
        .map(|i| {
            let rule = if !small(area[i]) {
                TargetRule::Size
            } else if spaced(i, area[i], &area) {
                TargetRule::Spacing
            } else {
                TargetRule::Short
            };
            Laid {
                id: live[i].0.id.clone(),
                art: live[i].1,
                area: area[i],
                rule,
            }
        })
        .collect()
}

/// Where `a` and `b` meet, if they do.
pub(super) fn meet(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = (
        (a[0] + a[2]).min(b[0] + b[2]),
        (a[1] + a[3]).min(b[1] + b[3]),
    );
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// `r` less `a`: the parts of `r` above, below, left and right of `a`.
pub(super) fn minus(r: [i32; 4], a: [i32; 4]) -> Vec<[i32; 4]> {
    let Some(i) = meet(r, a) else {
        return vec![r];
    };
    let (r1, r2, i1, i2) = (r[0] + r[2], r[1] + r[3], i[0] + i[2], i[1] + i[3]);
    [
        [r[0], r[1], r[2], i[1] - r[1]],
        [r[0], i2, r[2], r2 - i2],
        [r[0], i[1], i[0] - r[0], i[3]],
        [i1, i[1], r1 - i1, i[3]],
    ]
    .into_iter()
    .filter(|p| p[2] > 0 && p[3] > 0)
    .collect()
}

/// The target `l` serves its node: none for a node already 24 x 24 that
/// is not a grip (a grip's band is its pointer area whatever its art's
/// size, [`band`]) and keeps its art box (one [`under`] cuts serves it).
fn serve(l: &Laid, grip: bool) -> Option<NodeTarget> {
    if !grip && !small(l.art) && l.area == l.art {
        return None;
    }
    let (a, g) = (l.art, l.area);
    Some(NodeTarget {
        area: [g[0] - a[0], g[1] - a[1], g[2], g[3]],
        art: [a[0] - g[0], a[1] - g[1], a[2], a[3]],
        rule: l.rule,
    })
}

fn set(nodes: &mut [SkinNode], laid: &[Laid]) {
    for n in nodes {
        let grip = n.frame_node();
        n.target = laid
            .iter()
            .find(|l| l.id == n.id)
            .and_then(|l| serve(l, grip));
        set(&mut n.children, laid);
    }
}

/// The window (width, height) `layout`'s tree is laid in at its minimum:
/// the frame grown by its outsets, as the stylesheet lays it.
#[must_use]
pub fn window(layout: &WindowLayout) -> (i32, i32) {
    let o = layout.outsets;
    (
        i32::from(layout.min_width) + i32::from(o.left) + i32::from(o.right),
        i32::from(layout.min_height) + i32::from(o.top) + i32::from(o.bottom),
    )
}

/// Sets the target of every small interactive node in `layout`'s tree and
/// popups (see the module doc), laid at the window's minimum.
pub fn nodes(layout: &mut WindowLayout) {
    let at = window(layout);
    if let Some(tree) = layout.tree.as_mut() {
        lay(tree, at);
    }
    let min_height = layout.min_height;
    for popup in layout.popups.values_mut() {
        let at = window(&super::popup::frame(popup, min_height));
        lay(&mut popup.tree, at);
    }
}

/// Sets the targets of `tree` laid in `at`, and serves each grip the
/// triggers it takes from the system-menu strips it lies over
/// ([`Grip::takes`]).
fn lay(tree: &mut SkinTree, at: (i32, i32)) {
    let mut laid = targets(tree, at);
    let grips = grips(tree, at, &laid);
    let takes: Vec<(String, Vec<NodeTrigger>)> = grips
        .iter()
        .filter(|g| !g.takes.is_empty())
        .map(|g| (g.laid.id.clone(), g.takes.clone()))
        .collect();
    laid.extend(grips.into_iter().map(|g| g.laid));
    set(&mut tree.root, &laid);
    take(&mut tree.root, &takes);
}

/// Adds to each grip in `nodes` the triggers it takes, once.
fn take(nodes: &mut [SkinNode], takes: &[(String, Vec<NodeTrigger>)]) {
    for n in nodes {
        if let Some((_, ts)) = takes.iter().find(|(id, _)| *id == n.id) {
            for t in ts {
                if !n.triggers.contains(t) {
                    n.triggers.push(t.clone());
                }
            }
        }
        take(&mut n.children, takes);
    }
}
