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
//!   its part serves its own. It is not an edge grip (its own box takes no pointer events; its
//!   `::before` grip area stays, [`super::validate::grip_area`]) and is not
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
//! - Its rule: `Size` when the box is at least 24 x 24; else `Spacing` when
//!   the 24 px circle centred on its box meets no neighbour's target box and
//!   no undersized neighbour's circle (the criterion's spacing exception;
//!   a box cut at a neighbour touches it, so its art box is tried as the
//!   undersized target instead);
//! - A listening node the pack lays over its art (not a neighbour) keeps
//!   its centre: the box is cut back to the art's edge on that side
//!   ([`yield_to`]), so a press at that node's centre reaches it.
//!   else `Short`, neither holds: a pack-layout finding the Host still serves
//!   (the box it could grow to) and reports.
//! - A node already at least 24 x 24 is unchanged and serves no target.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::NodeTrigger;
use rekindle_types::dto::skin_tree::{NodeRole, NodeTarget, SkinNode, SkinTree, TargetRule};

use super::validate::{hit_area, overlap, text_sized, Apart, HIT};

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
fn side(a: [i32; 4], b: [i32; 4], axis: usize) -> Option<(bool, i32)> {
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

/// Cuts `g` (the target box of the node at `a`) back to `a`'s edge on the
/// side where the centre of `b`, a listening node the pack lays over or
/// under `a`'s art, lies outside `a`: a target box never takes a press
/// meant for another listening node's art (Symbiosis's `StatusChanger` on
/// its `Status` field). Where `a`'s art holds that centre, the pack's
/// stacking decides (rule 4).
fn yield_to(g: &mut [i32; 4], a: [i32; 4], b: [i32; 4]) {
    for axis in 0..2 {
        let c = 2 * b[axis] + b[axis + 2];
        let (lo, hi) = (a[axis], a[axis] + a[axis + 2]);
        let other = 1 - axis;
        let across = 2 * g[other] <= 2 * b[other] + b[other + 2]
            && 2 * b[other] + b[other + 2] < 2 * (g[other] + g[other + 2]);
        if !across {
            continue;
        }
        if c < 2 * lo && 2 * g[axis] <= c {
            g[axis + 2] -= lo - g[axis];
            g[axis] = lo;
        } else if c >= 2 * hi && c < 2 * (g[axis] + g[axis + 2]) {
            g[axis + 2] = hi - g[axis];
        }
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
    let near = |i: usize, j: usize| {
        i != j && !apart.apart(&live[i].0.id, &live[j].0.id) && !overlap(live[i].1, live[j].1)
    };
    let area: Vec<[i32; 4]> = (0..live.len())
        .map(|i| {
            let mut g = want[i];
            for j in (0..live.len()).filter(|&j| near(i, j)) {
                clip(&mut g, live[i].1, live[j].1, want[j]);
            }
            let laid_over = |j: usize| {
                i != j
                    && !apart.apart(&live[i].0.id, &live[j].0.id)
                    && overlap(live[i].1, live[j].1)
            };
            for j in (0..live.len()).filter(|&j| laid_over(j)) {
                yield_to(&mut g, live[i].1, live[j].1);
            }
            g
        })
        .collect();
    // The spacing exception's circle: 24 px across, 12 px (24 half
    // pixels) from its centre.
    let r2 = i64::from(HIT) * i64::from(HIT);
    let spaced = |i: usize, own: [i32; 4], area: &[[i32; 4]]| {
        (0..live.len())
            .filter(|&j| near(i, j))
            .all(|j| far(own, area[j]) >= r2 && !(small(area[j]) && centres(own, area[j]) < 4 * r2))
    };
    // A box cut short of 24 px meets its neighbour's, so its circle never
    // clears it: the node then keeps its art box as its (undersized)
    // target when that clears every neighbour (the spacing exception on
    // the art, as the criterion measures an undersized target).
    let area: Vec<[i32; 4]> = (0..live.len())
        .map(|i| {
            let art = live[i].1;
            if small(area[i]) && !spaced(i, area[i], &area) && spaced(i, art, &area) {
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

/// The target `l` serves its node: none for a node already 24 x 24.
fn serve(l: &Laid) -> Option<NodeTarget> {
    if !small(l.art) {
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
        n.target = laid.iter().find(|l| l.id == n.id).and_then(serve);
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
        let laid = targets(tree, at);
        set(&mut tree.root, &laid);
    }
    let min_height = layout.min_height;
    for popup in layout.popups.values_mut() {
        let at = window(&super::popup::frame(popup, min_height));
        let laid = targets(&popup.tree, at);
        set(&mut popup.tree.root, &laid);
    }
}
