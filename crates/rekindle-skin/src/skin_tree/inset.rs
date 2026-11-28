//! The inset of a text field (plan SK5, the contract with the shell): a
//! pack may lay an interactive sibling over a text field's box. Symbiosis
//! stretches `UsernameContainer` (the name field) across its top and lays
//! `ButtonNotificationsTab`, the 26 x 24 notifications gear, over its end,
//! so a long name ran under the gear with no ellipsis to show it was cut.
//!
//! The Host serves the field `inset: {left, right}`, how far the siblings
//! that take input reach into it from each edge, in the tree laid at the
//! window's served minimum, and the shell lays the field's text and its
//! ellipsis between them; the sibling keeps its place and stays pressable.
//!
//! - A text field is a node of an editable text role: the name, status
//!   and room fields ([`is_text`]).
//! - A covering sibling is a node beside it in the same parent that takes
//!   input ([`SkinNode::interactive`]: a button, tab, menu title or status
//!   list) and is not itself a text field.
//! - Plan SK13: the field's text also stops at every other node's press
//!   area that meets it, anywhere in the tree (the target box
//!   [`super::target::targets`] lays, else the node's box), except its own
//!   ancestors and descendants, a resize grip, a frame node and another
//!   view's node, even where the pack overlaps the two arts: Alloy's 205 px
//!   name field stops where its status field's press area starts (x 171),
//!   Alloy's status field at `DownArrow`'s target (x 336, 3 px before its
//!   art), Simple-X's name field at the title bar's `MinimizeBox` target
//!   (x 334). The field and the other node keep their places.
//! - A resize grip (a pointer trigger that resizes, [`SkinNode::resize_edge`])
//!   draws nothing over the field and never covers it; nor does a node of
//!   another view (`shows_with` naming another tab: never shown with it).
//! - A sibling meeting the field's box (both axes, a positive area) covers
//!   it from the edge nearer the sibling's centre: from the right
//!   `field right - sibling left`, from the left `sibling right - field
//!   left`, at most the field's width. The largest reach on each side wins.
//! - A field no sibling meets serves no inset.
//!
//! - Plan SK15: the inset is fixed px from each edge, so where the field
//!   stretches with the window but the press area its text stops at does
//!   not (Alloy's name field, `Stretch`, under its status field's press
//!   area at x 171), a constant `right` lets the text run under that area
//!   at a wider window. The tree is also laid `INSET_PROBE_WIDTH` px wider;
//!   when the stop moves otherwise than the field's far edge, the inset
//!   carries `max_width`, the text's width from its start to the stop at
//!   the minimum (Alloy: 151), and the shell caps the text there.
//!
//! Measured on the built-ins: Symbiosis's name field `[7, 82, 330, 13]`
//! under its gear `[309, 77, 26, 24]` serves `{left: 0, right: 28}` (the
//! resize strip `ResizeR` at x 332 reaches 5 px, under the gear's 28).

use std::collections::HashMap;

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{NodeInset, NodeRole, SkinNode, SkinTree, ViewKey};

use super::target::listens;
use super::titles::{each_laid, laid_window, place};

/// An editable text role (the name, status and room fields).
pub(super) const fn is_text(role: NodeRole) -> bool {
    matches!(
        role,
        NodeRole::NameField | NodeRole::StatusText | NodeRole::RoomName
    )
}

/// How far `by` reaches into `field` (both `[x, y, w, h]`) from each
/// side, `[left, right]`, or `None` when they do not meet.
fn reach(field: [i32; 4], by: [i32; 4]) -> Option<[i32; 2]> {
    let across = field[0].max(by[0]) < (field[0] + field[2]).min(by[0] + by[2]);
    let down = field[1].max(by[1]) < (field[1] + field[3]).min(by[1] + by[3]);
    if !(across && down) {
        return None;
    }
    let right_side = 2 * by[0] + by[2] > 2 * field[0] + field[2];
    let (left, right) = if right_side {
        (0, field[0] + field[2] - by[0])
    } else {
        (by[0] + by[2] - field[0], 0)
    };
    Some([left.min(field[2]), right.min(field[2])])
}

/// A press area a field's text stops at (plan SK13): the node's id, its
/// ancestors' ids, the view it shows with, its press area in the window.
struct Press {
    id: String,
    up: Vec<String>,
    shows: Option<ViewKey>,
    area: [i32; 4],
}

fn presses(
    nodes: &[SkinNode],
    parent: [i32; 4],
    (up, view): (&[String], Option<ViewKey>),
    laid: &HashMap<String, [i32; 4]>,
    out: &mut Vec<Press>,
) {
    for n in nodes {
        let b = place(n, parent);
        let shows = n.shows_with.or(view);
        if listens(n) && !n.frame_node() && b[2] > 0 && b[3] > 0 {
            out.push(Press {
                id: n.id.clone(),
                up: up.to_vec(),
                shows,
                area: laid.get(&n.id).copied().unwrap_or(b),
            });
        }
        let mut own = up.to_vec();
        own.push(n.id.clone());
        presses(&n.children, b, (&own, shows), laid, out);
    }
}

/// The reach `[left, right]` into the field `n` at `b` (ancestors `up`,
/// shown with `shows`) of every press area that stops its text.
fn press_reach(
    n: &SkinNode,
    b: [i32; 4],
    (up, shows): (&[String], Option<ViewKey>),
    all: &[Press],
) -> Option<[i32; 2]> {
    all.iter()
        .filter(|p| {
            let apart = p.shows.is_some() && shows.is_some() && p.shows != shows;
            let kin = p.id == n.id || p.up.contains(&n.id) || up.contains(&p.id);
            !(apart || kin)
        })
        .filter_map(|p| reach(b, p.area))
        .reduce(|[l, r], [a, c]| [l.max(a), r.max(c)])
}

/// The inset of the text field at `field` among `siblings` laid at
/// `boxes`, starting from the press areas' reach `from`.
fn inset_of(
    field: usize,
    siblings: &[SkinNode],
    boxes: &[[i32; 4]],
    from: Option<[i32; 2]>,
) -> Option<NodeInset> {
    let mut sides: Option<[i32; 2]> = from;
    for (i, s) in siblings.iter().enumerate() {
        let apart = s.shows_with.is_some()
            && siblings[field].shows_with.is_some()
            && s.shows_with != siblings[field].shows_with;
        if i == field || apart || is_text(s.role) || !s.interactive() || s.resize_edge().is_some() {
            continue;
        }
        if let Some([left, right]) = reach(boxes[field], boxes[i]) {
            let [l, r] = sides.unwrap_or([0, 0]);
            sides = Some([l.max(left), r.max(right)]);
        }
    }
    let px = |v: i32| u16::try_from(v.max(0)).unwrap_or(u16::MAX);
    sides.map(|[left, right]| NodeInset {
        left: px(left),
        right: px(right),
        max_width: None,
    })
}

/// Each text field's box and inset in a laid tree, by id.
type Fields = HashMap<String, ([i32; 4], Option<NodeInset>)>;

fn walk(
    nodes: &mut [SkinNode],
    parent: [i32; 4],
    (up, view): (&[String], Option<ViewKey>),
    all: &[Press],
    fields: &mut Fields,
) {
    let boxes: Vec<[i32; 4]> = nodes.iter().map(|n| place(n, parent)).collect();
    let insets: Vec<Option<NodeInset>> = (0..nodes.len())
        .map(|i| {
            let shows = nodes[i].shows_with.or(view);
            is_text(nodes[i].role)
                .then(|| {
                    let from = press_reach(&nodes[i], boxes[i], (up, shows), all);
                    inset_of(i, nodes, &boxes, from)
                })
                .flatten()
        })
        .collect();
    for ((n, inset), b) in nodes.iter_mut().zip(insets).zip(boxes) {
        n.inset = inset;
        if is_text(n.role) {
            fields.insert(n.id.clone(), (b, inset));
        }
        let shows = n.shows_with.or(view);
        let mut own = up.to_vec();
        own.push(n.id.clone());
        walk(&mut n.children, b, (&own, shows), all, fields);
    }
}

/// How much wider than its served minimum the window is laid to learn
/// whether a field's text stop moves with the field's far edge.
const INSET_PROBE_WIDTH: i32 = 200;

/// Where the text of a field at `b` with `inset` stops (window x).
fn stop(b: [i32; 4], inset: Option<NodeInset>) -> i32 {
    b[0] + b[2] - inset.map_or(0, |i| i32::from(i.right))
}

/// Sets each field's `max_width` (plan SK15): laid at the minimum (`at`)
/// and `INSET_PROBE_WIDTH` px wider (`wide`), a field whose text stop does
/// not move with its far edge serves the width from its text start to the
/// stop at the minimum.
fn cap(nodes: &mut [SkinNode], at: &Fields, wide: &Fields) {
    for n in nodes {
        if let (Some(inset), Some(&(b, _)), Some(&(w, wide_inset))) =
            (n.inset.as_mut(), at.get(&n.id), wide.get(&n.id))
        {
            let edge_moves = (w[0] + w[2]) - (b[0] + b[2]);
            let stop_moves = stop(w, wide_inset) - stop(b, Some(*inset));
            if stop_moves != edge_moves {
                let text = stop(b, Some(*inset)) - b[0] - i32::from(inset.left);
                inset.max_width = Some(u16::try_from(text.max(0)).unwrap_or(u16::MAX));
            }
        }
        cap(&mut n.children, at, wide);
    }
}

/// Sets the insets of `tree` laid in `window`, and their `max_width`.
fn lay_tree(tree: &mut SkinTree, window: (i32, i32)) {
    let mut wide = tree.clone();
    let wide_fields = lay_at(&mut wide, (window.0 + INSET_PROBE_WIDTH, window.1));
    let fields = lay_at(tree, window);
    cap(&mut tree.root, &fields, &wide_fields);
}

/// Sets the insets of `tree` laid in `window`; its text fields' boxes and
/// insets by id.
fn lay_at(tree: &mut SkinTree, window: (i32, i32)) -> Fields {
    let laid: HashMap<String, [i32; 4]> = super::target::targets(tree, window)
        .into_iter()
        .map(|l| (l.id, l.area))
        .collect();
    let at = laid_window(tree.outsets, window);
    let mut all = Vec::new();
    presses(&tree.root, at, (&[], None), &laid, &mut all);
    let mut fields = Fields::new();
    walk(&mut tree.root, at, (&[], None), &all, &mut fields);
    fields
}

/// Sets the inset of every text field in `layout`'s tree and popups (see
/// the module doc), laid at the window's minimum size.
pub fn fields(layout: &mut WindowLayout) {
    each_laid(layout, lay_tree);
}
