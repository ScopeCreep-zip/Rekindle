//! Plan SK12: the served boxes no target grows over, the views (the app's
//! content) and every served text box, and the cut that keeps a target out
//! of them (used by `target::targets`).

use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree};

use super::target::side;
use super::validate::overlap;

/// The boxes of `tree` laid in `window` no target grows over: the views
/// (`true`) and every served text box (its rows, at least one).
pub(super) fn boxes(tree: &SkinTree, window: (i32, i32)) -> Vec<(&str, [i32; 4], bool)> {
    tree.boxes(window.0, window.1)
        .into_iter()
        .filter_map(|(n, b)| {
            if n.role == NodeRole::Views {
                Some((n.id.as_str(), b, true))
            } else {
                let text = super::text_box::draws_text(n) && b[2] > 0;
                text.then_some((n.id.as_str(), [b[0], b[1], b[2], b[3].max(1)], false))
            }
        })
        .collect()
}

/// Whether the node `id` of `root` is `n`, lies under it, or holds it
/// (a text-drawn title over its button part).
pub(super) fn kin(root: &[SkinNode], n: &SkinNode, id: &str) -> bool {
    fn holds(n: &SkinNode, id: &str) -> bool {
        n.id == id || n.children.iter().any(|c| holds(c, id))
    }
    fn find<'a>(nodes: &'a [SkinNode], id: &str) -> Option<&'a SkinNode> {
        nodes
            .iter()
            .find_map(|m| (m.id == id).then_some(m).or_else(|| find(&m.children, id)))
    }
    holds(n, id) || find(root, id).is_some_and(|m| holds(m, &n.id))
}

/// Cuts `g` (the target box of the node at `a`) back so it meets `o` no
/// further than `a`'s art does: `o` is the views (`views`, the app's
/// content: Simple-X's menu buttons end 5 px into it, over the buddy
/// search field, so their targets stop at the art) or a served text box
/// (Onyx's `Title` starts 2 px inside its `Logo`'s 24 px target). The cut
/// is along the axis `a` and `o` are apart on (the larger gap, x on a
/// tie), the whole gap `a`'s: `o` serves no target into it. An art laid
/// over a text keeps the pack's stacking; an art over the views is cut on
/// the axis it sticks out of them on; one inside them is theirs.
pub(super) fn keep_out(g: &mut [i32; 4], a: [i32; 4], o: [i32; 4], views: bool) {
    if !overlap(*g, o) {
        return;
    }
    let (axis, after) = match (side(a, o, 0), side(a, o, 1)) {
        (Some((ax, gx)), Some((ay, gy))) if gy > gx => (1, ay),
        (Some((ax, _)), _) => (0, ax),
        (None, Some((ay, _))) => (1, ay),
        (None, None) if views => {
            let out = (0..2).rev().find_map(|axis| {
                if a[axis] < o[axis] {
                    Some((axis, true))
                } else if a[axis] + a[axis + 2] > o[axis] + o[axis + 2] {
                    Some((axis, false))
                } else {
                    None
                }
            });
            let Some(out) = out else { return };
            out
        }
        (None, None) => return,
    };
    // The art's own reach into `o` serves a target only across the art's
    // span: a target wider than its art (Onyx's menu titles, 19 px over a
    // 12 px art that ends 1 px into the views) stops at `o`'s edge.
    let c = 1 - axis;
    let within = g[c] >= a[c] && g[c] + g[c + 2] <= a[c] + a[c + 2];
    if after {
        let end = if within {
            o[axis].max(a[axis] + a[axis + 2])
        } else {
            o[axis]
        };
        g[axis + 2] = g[axis + 2].min(end - g[axis]);
    } else {
        let edge = o[axis] + o[axis + 2];
        let start = if within { edge.min(a[axis]) } else { edge };
        if g[axis] < start {
            g[axis + 2] -= start - g[axis];
            g[axis] = start;
        }
    }
}
